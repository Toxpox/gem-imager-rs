//! Flash Linux Os Images to SD Cards with optioinal post-install customization.
//!
//! Post-install customization is only available for [BeagleBoard.org] images
//!
//! [BeagleBoard.org]: https://www.beagleboard.org/

mod cloud_init;

use bb_helper::cancel::CancellationToken;
use std::{borrow::Cow, fmt::Display, path::PathBuf};

use crate::common::{BBFlasherTarget, DownloadFlashingStatus};

/// SD Card
#[derive(Debug, Hash, PartialEq, Eq, Clone)]
pub struct Target(bb_flasher_sd::Device);

impl Target {
    fn destinations_internal(filter: bool) -> Vec<Self> {
        bb_flasher_sd::devices(filter)
            .into_iter()
            .map(Self)
            .collect()
    }

    /// SD Card size in bytes
    pub const fn size(&self) -> u64 {
        self.0.size
    }

    pub fn path(&self) -> &std::path::Path {
        &self.0.path
    }
}

impl Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.name.fmt(f)
    }
}

impl TryFrom<PathBuf> for Target {
    type Error = std::io::Error;

    fn try_from(value: PathBuf) -> Result<Self, Self::Error> {
        Self::destinations_internal(false)
            .into_iter()
            .find(|x| x.0.path == value)
            .ok_or(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "SD Card target not found",
            ))
    }
}

impl BBFlasherTarget for Target {
    const FILE_TYPES: &[&str] = &["img", "xz", "qcow2"];

    fn destinations(filter: bool) -> Vec<Self> {
        Self::destinations_internal(filter)
    }

    fn identifier(&self) -> Cow<'_, str> {
        self.0.path.to_string_lossy()
    }
}

/// Linux Image post-install customization options.
///
/// Each entry is a file to place on the boot partition. Entries carrying secrets are marked so the
/// SD backend reads them back off the card and compares them, instead of trusting the write.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FlashingSdLinuxConfig(Vec<(Box<str>, Box<[u8]>, Verification)>);

/// Whether a customization file is read back off the card after it is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Verification {
    /// Trust the filesystem write, as the BeagleBoard paths have always done.
    None,
    /// Re-open the partition, read the file back and compare it byte for byte.
    ReadBack,
}

fn sysconf_w(sysconf: &mut Vec<u8>, key: &str, value: &str) {
    sysconf.extend(key.as_bytes());
    sysconf.extend(b"=");
    sysconf.extend(value.as_bytes());
    sysconf.extend(b"\n");
}

impl FlashingSdLinuxConfig {
    pub fn sysconfig(
        hostname: Option<Box<str>>,
        timezone: Option<Box<str>>,
        keymap: Option<Box<str>>,
        user: Option<(Box<str>, Box<str>)>,
        wifi: Option<(Box<str>, Box<str>)>,
        ssh: Option<Box<str>>,
        usb_enable_dhcp: Option<bool>,
    ) -> Self {
        let mut content = Vec::<u8>::new();

        if let Some(h) = hostname {
            sysconf_w(&mut content, "hostname", &h);
        }
        if let Some(tz) = timezone {
            sysconf_w(&mut content, "timezone", &tz);
        }
        if let Some(k) = keymap {
            sysconf_w(&mut content, "keymap", &k);
        }
        if let Some((u, p)) = user {
            sysconf_w(&mut content, "user_name", &u);
            sysconf_w(&mut content, "user_password", &p);
        }
        if let Some(x) = ssh {
            sysconf_w(&mut content, "user_authorized_key", &x);
        }
        if Some(true) == usb_enable_dhcp {
            sysconf_w(&mut content, "usb_enable_dhcp", "yes");
        }

        match wifi {
            Some((ssid, psk)) => {
                sysconf_w(&mut content, "iwd_psk_file", &format!("{ssid}.psk"));

                Self(vec![
                    (
                        "sysconf.txt".to_string().into(),
                        content.into(),
                        Verification::None,
                    ),
                    (
                        format!("services/{ssid}.psk").into(),
                        format!("[Security]\nPassphrase={psk}\n\n[Settings]\nAutoConnect=true")
                            .into_bytes()
                            .into(),
                        Verification::None,
                    ),
                ])
            }
            None => Self(vec![(
                "sysconf.txt".to_string().into(),
                content.into(),
                Verification::None,
            )]),
        }
    }

    /// Customization for T3 GemStone images: the `config.ini` file `gem-first-boot` consumes.
    ///
    /// The bytes come from [`crate::t3_gem_init`], which is the only place allowed to build them —
    /// the file is `source`d as root on the board, so it is never assembled by concatenation here.
    ///
    /// It is written with [`Verification::ReadBack`]: a `config.ini` that silently failed to land
    /// produces a board with the factory password and no network, which looks like a successful
    /// flash until the user tries to log in.
    #[cfg(feature = "t3_gem_init")]
    pub fn t3_gem_init(
        config: &crate::t3_gem_init::T3GemInitConfig,
    ) -> Result<Self, crate::t3_gem_init::T3GemInitError> {
        let content = config.serialize()?;

        Ok(Self(vec![(
            crate::t3_gem_init::CONFIG_FILE_NAME.to_string().into(),
            content.to_vec().into(),
            Verification::ReadBack,
        )]))
    }

    pub fn cloud_init(
        hostname: Option<Box<str>>,
        timezone: Option<Box<str>>,
        keymap: Option<Box<str>>,
        user: Option<(Box<str>, Box<str>)>,
        wifi: Option<(Box<str>, Box<str>)>,
        ssh: Option<Box<str>>,
    ) -> Self {
        let data = cloud_init::CloudInitConfig::new(hostname, timezone, keymap, user, wifi, ssh);
        Self(vec![(
            "cloud-init".to_string().into(),
            data.to_file_data(),
            Verification::None,
        )])
    }

    pub fn generic_file(file_name: Box<str>, file_content: Box<str>) -> Self {
        Self(vec![(
            file_name,
            file_content.into_boxed_bytes(),
            Verification::None,
        )])
    }

    pub const fn none() -> Self {
        Self(Vec::new())
    }
}

impl Extend<Self> for FlashingSdLinuxConfig {
    fn extend<T: IntoIterator<Item = Self>>(&mut self, iter: T) {
        self.0.extend(iter.into_iter().flat_map(|x| x.0));
    }
}

/// Flasher to format SD Cards
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FormatFlasher(PathBuf);

impl FormatFlasher {
    pub fn new(p: Target) -> Self {
        Self(p.0.path)
    }

    pub fn flash(self) -> anyhow::Result<()> {
        bb_flasher_sd::format(self.0.as_path()).map_err(Into::into)
    }
}

/// Map the SD backend's stages onto the front-end progress vocabulary.
///
/// When the destination is a plain file the "write" stage is really the download/extract that
/// produced it, which is why it is reported as `DownloadingProgress` there. The verify stage keeps
/// its own fraction in both cases — it is a separate pass over the whole image, not a tail of the
/// write.
const fn translate_status(
    status: bb_flasher_sd::Status,
    is_file_dest: bool,
) -> DownloadFlashingStatus {
    match status {
        bb_flasher_sd::Status::Preparing => DownloadFlashingStatus::Preparing,
        bb_flasher_sd::Status::Writing(x) => {
            if is_file_dest {
                DownloadFlashingStatus::DownloadingProgress(x)
            } else {
                DownloadFlashingStatus::FlashingProgress(x)
            }
        }
        bb_flasher_sd::Status::Verifying(x) => DownloadFlashingStatus::Verifying(x),
        bb_flasher_sd::Status::Customizing => DownloadFlashingStatus::Customizing,
    }
}

#[cfg(test)]
mod status_tests {
    use super::*;

    /// The extract gate has to fail the **flash**, not merely the read.
    ///
    /// the integrity policy puts the extracted digest between the decoder and the writer, and the
    /// verifier only settles when the decoder reports EOF. That makes the whole guarantee depend on
    /// the writer reading all the way to EOF rather than stopping once it has the declared number of
    /// bytes. Nothing else asserts that, and if it ever regressed the gate would silently never fire
    /// — the flash would report success for an image the catalog never published.
    #[test]
    fn a_declared_gate_mismatch_fails_the_whole_flash() {
        use crate::img::{ExtractGate, ExtractedIntegrity, OsImage};
        use std::io::Write as _;

        let payload = vec![0xa5u8; 64 * 1024];

        let mut archive = tempfile::NamedTempFile::new().unwrap();
        let mut encoder = liblzma::write::XzEncoder::new(Vec::new(), 6);
        encoder.write_all(&payload).unwrap();
        archive.write_all(&encoder.finish().unwrap()).unwrap();
        archive.flush().unwrap();

        let dst = tempfile::NamedTempFile::new().unwrap();
        let size = payload.len() as u64;
        // Right size, wrong digest: exactly the "a different image was downloaded" case, which the
        // post-write read-back cannot detect because the card faithfully keeps what it was handed.
        let gate = ExtractGate::Declared(ExtractedIntegrity::new(size, [0u8; 32]));
        let path = archive.path().to_path_buf();

        let err = Flasher::with_file_dest(
            move || Ok((OsImage::from_path(&path, gate)?, size)),
            dst.path().to_path_buf(),
            FlashingSdLinuxConfig::none(),
        )
        .flash(None, None)
        .expect_err("an image that fails the extracted digest must not flash successfully");

        // Asserted on the source chain, not on `err.to_string()`. The SD backend wraps the decoder's
        // `io::Error` in its catch-all `IoError` variant, whose own message is "Unknown Error during
        // IO" — so the outermost text says nothing about integrity and the reason lives one level
        // down. The front-end renders this same chain (`{e:#}`); asserting on the chain here is what
        // keeps the two in step.
        let chain = format!("{err:#}");
        assert!(
            chain.contains("integrity"),
            "the integrity failure must survive into the reported chain, got {chain}"
        );
    }

    /// The same pipeline with the true digest completes, so the test above fails for the declared
    /// reason rather than because this path never works at all.
    #[test]
    fn a_matching_declared_gate_flashes_to_completion() {
        use crate::img::{ExtractGate, ExtractedIntegrity, OsImage};
        use sha2::{Digest as _, Sha256};
        use std::io::Write as _;

        let payload = vec![0xa5u8; 64 * 1024];
        let sha256: [u8; 32] = Sha256::digest(&payload).into();

        let mut archive = tempfile::NamedTempFile::new().unwrap();
        let mut encoder = liblzma::write::XzEncoder::new(Vec::new(), 6);
        encoder.write_all(&payload).unwrap();
        archive.write_all(&encoder.finish().unwrap()).unwrap();
        archive.flush().unwrap();

        let dst = tempfile::NamedTempFile::new().unwrap();
        let size = payload.len() as u64;
        let gate = ExtractGate::Declared(ExtractedIntegrity::new(size, sha256));
        let path = archive.path().to_path_buf();

        Flasher::with_file_dest(
            move || Ok((OsImage::from_path(&path, gate)?, size)),
            dst.path().to_path_buf(),
            FlashingSdLinuxConfig::none(),
        )
        .flash(None, None)
        .expect("a matching image must flash");
    }

    #[test]
    fn writing_to_a_device_is_flashing_and_to_a_file_is_downloading() {
        assert_eq!(
            translate_status(bb_flasher_sd::Status::Writing(0.5), false),
            DownloadFlashingStatus::FlashingProgress(0.5)
        );
        assert_eq!(
            translate_status(bb_flasher_sd::Status::Writing(0.5), true),
            DownloadFlashingStatus::DownloadingProgress(0.5)
        );
    }

    /// Verification must never be folded back into the write bar: the user has to be able to see
    /// that a distinct read-back pass ran.
    #[test]
    fn verification_keeps_its_own_stage_and_fraction() {
        assert_eq!(
            translate_status(bb_flasher_sd::Status::Verifying(0.25), false),
            DownloadFlashingStatus::Verifying(0.25)
        );
        assert_eq!(
            translate_status(bb_flasher_sd::Status::Verifying(0.25), true),
            DownloadFlashingStatus::Verifying(0.25)
        );
    }

    #[test]
    fn preparing_and_customizing_pass_through() {
        assert_eq!(
            translate_status(bb_flasher_sd::Status::Preparing, false),
            DownloadFlashingStatus::Preparing
        );
        assert_eq!(
            translate_status(bb_flasher_sd::Status::Customizing, false),
            DownloadFlashingStatus::Customizing
        );
    }
}

/// Flasher of flashing Os Images to SD Card
///
/// # Supported Images
///
/// - img: Raw images
/// - xz: Xz compressed raw images
#[derive(Debug, Clone)]
pub struct Flasher<I> {
    img: I,
    dst: bb_flasher_sd::Destination,
    customization: FlashingSdLinuxConfig,
}

impl<I> Flasher<I> {
    pub fn new(img: I, dst: Target, customization: FlashingSdLinuxConfig) -> Self {
        Self {
            img,
            dst: bb_flasher_sd::Destination::SdCard(dst.0.path.into_boxed_path()),
            customization,
        }
    }

    pub fn with_file_dest(img: I, dst: PathBuf, customization: FlashingSdLinuxConfig) -> Self {
        Self {
            img,
            dst: bb_flasher_sd::Destination::File(dst.into_boxed_path()),
            customization,
        }
    }

    const fn is_file_dest(&self) -> bool {
        matches!(self.dst, bb_flasher_sd::Destination::File(_))
    }
}

impl<I> Flasher<I>
where
    I: FnOnce() -> std::io::Result<(crate::img::OsImage, u64)> + Send,
{
    pub fn flash(
        self,
        chan: Option<std::sync::mpsc::SyncSender<DownloadFlashingStatus>>,
        cancel: Option<CancellationToken>,
    ) -> anyhow::Result<()> {
        let is_file_dest = self.is_file_dest();
        let customization = if self.customization.0.is_empty() {
            vec![]
        } else {
            let content = self.customization.0.into_iter().map(|(p, d, v)| {
                let content = match v {
                    Verification::None => bb_flasher_sd::ContentType::DataAppend(d),
                    Verification::ReadBack => bb_flasher_sd::ContentType::VerifiedData(d),
                };
                (p, content)
            });
            vec![bb_flasher_sd::Customization {
                partition: bb_flasher_sd::ParitionType::Boot,
                content,
            }]
        }
        .into_iter();

        let tx = match chan {
            Some(chan) => {
                let (tx, rx) = std::sync::mpsc::sync_channel(2);
                std::thread::spawn(move || {
                    // Should run until tx is dropped, i.e. flasher task is done.
                    // If it is aborted, then cancel should be dropped, thereby signaling the flasher task to abort
                    while let Ok(x) = rx.recv() {
                        let _ = chan.try_send(translate_status(x, is_file_dest));
                    }
                });

                Some(tx)
            }
            None => None,
        };

        bb_flasher_sd::flash(self.img, self.dst, tx, customization, cancel).map_err(Into::into)
    }
}
