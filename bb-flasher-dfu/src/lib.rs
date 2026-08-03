//! Fail-closed T3-GEM-O1 DFU backend.
//!
//! USB discovery and control transfers are isolated behind [`DfuTransport`]. The protocol runner
//! consumes typed stages, keeps the full physical USB topology across resets, streams raw eMMC
//! without loading it into RAM, and only succeeds with terminal DFU evidence.

mod model;
mod resolver;
mod state_machine;
mod transport;

use std::{
    io::{self, Seek as _, Write as _},
    path::{Path, PathBuf},
    sync::mpsc,
};

use bb_helper::cancel::CancellationToken;
use rusb::UsbContext as _;
use sha2::{Digest as _, Sha256};
use thiserror::Error;

pub use bb_config::t3::{T3_DFU_PRODUCT_ID, T3_DFU_RECONNECT_TIMEOUT, T3_DFU_VENDOR_ID};
pub use model::{
    DfuDevice, DfuProgress, DfuStage, DfuStageInput, DfuStageKind, DfuState, DfuStatus,
    DfuTerminalEvidence, FlashReport, StageReport, TransportError, TransportErrorKind, UsbPath,
};
pub use resolver::{BootArtifactResolver, ResolvedBootArtifact};
pub use state_machine::flash_with_transport;
pub use transport::{DfuTransport, RusbTransport};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("DFU operation was cancelled before it completed")]
    Aborted,
    #[error("DFU transport error: {0}")]
    Transport(#[from] TransportError),
    #[error("no DFU device was found at {0}")]
    DeviceNotFound(UsbPath),
    #[error("{count} devices match {vendor_id:04x}:{product_id:04x}; choose one physical port")]
    AmbiguousDevice {
        vendor_id: u16,
        product_id: u16,
        count: usize,
    },
    #[error("timed out waiting for alt-setting `{alt_setting}` at {path}")]
    ReconnectTimeout { alt_setting: String, path: UsbPath },
    #[error("expected alt-setting `{expected}`; available: {available:?}")]
    WrongAltSetting {
        expected: String,
        available: Vec<String>,
    },
    #[error("invalid USB device identity: {0}")]
    InvalidDeviceIdentity(String),
    #[error("the device moved from {expected} to {actual} during re-enumeration")]
    DevicePathChanged { expected: UsbPath, actual: UsbPath },
    #[error("a DFU interface is already claimed")]
    InterfaceAlreadyClaimed,
    #[error("no DFU interface is claimed")]
    NoClaimedInterface,
    #[error("alt-setting `{alt_setting}` has no valid DFU transfer size")]
    MissingTransferSize { alt_setting: String },
    #[error("invalid DFU transfer size {0}")]
    InvalidTransferSize(usize),
    #[error("DFU GETSTATUS returned {0} bytes instead of 6")]
    MalformedStatus(usize),
    #[error("unexpected DFU state {state:?}, status {status}, while {context}")]
    UnexpectedState {
        context: &'static str,
        state: DfuState,
        status: u8,
    },
    #[error("timed out while waiting for {0}")]
    StatusTimeout(&'static str),
    #[error("invalid DFU plan: {0}")]
    InvalidPlan(String),
    #[error("stage `{artifact}` has no expected byte count")]
    MissingSize { artifact: String },
    #[error("DFU progress byte count overflow")]
    ProgressSizeOverflow,
    #[error("stage `{0}` is empty")]
    EmptyImage(String),
    #[error("failed to read stage `{artifact}`: {source}")]
    ImageRead {
        artifact: String,
        #[source]
        source: io::Error,
    },
    #[error("stage `{artifact}` ended early: expected {expected} bytes, read {actual}")]
    ShortImage {
        artifact: String,
        expected: u64,
        actual: u64,
    },
    #[error("stage `{artifact}` contains more than the expected {expected} bytes")]
    LongImage { artifact: String, expected: u64 },
    #[error("stage `{artifact}` SHA-256 mismatch")]
    StageHashMismatch {
        artifact: String,
        expected: [u8; 32],
        actual: [u8; 32],
    },
    #[error("short USB control write: expected {expected} bytes, wrote {actual}")]
    ShortUsbWrite { expected: usize, actual: usize },
    #[error("failed to transfer stage `{artifact}`: {source}")]
    StageTransfer {
        artifact: String,
        #[source]
        source: Box<Error>,
    },
    #[error("failed to send the final zero-length packet for `{artifact}`: {source}")]
    Zlp {
        artifact: String,
        #[source]
        source: Box<Error>,
    },
    #[error("device disconnected before dfuIDLE or dfuMANIFEST_WAIT_RST was observed")]
    DisconnectedBeforeTerminalState,
    #[error("final DFU detach failed: {source}")]
    FinalDetach {
        #[source]
        source: Box<Error>,
    },
    #[error("boot manifest download failed: {0}")]
    BootManifestDownload(#[source] bb_downloader::DownloadError),
    #[error("boot manifest is invalid: {0}")]
    BootManifest(#[source] bb_config::t3::BootManifestError),
    #[error("boot manifest JSON serialization failed: {0}")]
    BootManifestSerialize(#[source] serde_json::Error),
    #[error("boot artifact URL is invalid: {0}")]
    BootArtifactUrl(#[source] url::ParseError),
    #[error("boot artifact download failed: {0}")]
    BootArtifactDownload(#[source] bb_downloader::DownloadError),
    #[error("boot artifact resolver I/O failed: {0}")]
    ResolverIo(#[source] io::Error),
    #[error("no verified boot manifest is available (remote: {remote}; cache: {cache})")]
    NoVerifiedBootManifest {
        remote: Box<Error>,
        cache: Box<Error>,
    },
}

pub(crate) fn check_cancel(cancel: Option<&CancellationToken>) -> Result<()> {
    if cancel.is_some_and(CancellationToken::is_cancelled) {
        Err(Error::Aborted)
    } else {
        Ok(())
    }
}

/// Whether a listed device can actually be opened, and if not, why.
///
/// An inaccessible board must still be **listed**. Dropping it from the enumeration is what makes
/// the "assign WinUSB" and "install the udev rules" messages unreachable: the user cannot select a
/// destination that is not on the screen, so the write that would produce those errors never
/// starts and the board simply looks absent.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum DeviceAccess {
    /// The device opened; a write can be attempted.
    Available,
    /// The OS refused access — on Linux, the missing `uaccess`/udev rule.
    PermissionDenied,
    /// No usable driver is bound to the device — on Windows, WinUSB is not assigned.
    DriverMissing,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct Device {
    pub bus_num: u8,
    pub port_num: u8,
    pub port_path: Vec<u8>,
    pub vendor_id: u16,
    pub product_id: u16,
    pub name: String,
    /// Whether this device can be opened right now.
    pub access: DeviceAccess,
}

impl Device {
    pub fn physical_path(&self) -> UsbPath {
        UsbPath {
            bus: self.bus_num,
            ports: self.port_path.clone(),
        }
    }
}

/// Enumerate boards this backend can write to.
///
/// Restricted to the T3 DFU identity, always. A DFU-class filter is not equivalent: a printer, a
/// microcontroller dev board or a phone in fastboot can all present a DFU interface, and offering
/// one as a destination on a screen whose next button erases onboard storage is a wrong result,
/// not a permissive one. The `_show_all` toggle of [`crate::Device`]'s front-end deliberately does
/// not widen this — "show all destinations" exists to reveal *disks* the SD path hides, and there
/// is no corresponding hidden-but-valid DFU device.
///
/// Devices that cannot be opened are listed with their reason rather than dropped, so the front-end
/// can tell the user to assign WinUSB or install the udev rules.
pub fn devices(_show_all: bool) -> Vec<Device> {
    let Ok(context) = rusb::Context::new() else {
        return Vec::new();
    };
    let Ok(all) = context.devices() else {
        return Vec::new();
    };
    all.iter()
        .filter_map(|device| {
            let descriptor = device.device_descriptor().ok()?;
            if descriptor.vendor_id() != T3_DFU_VENDOR_ID
                || descriptor.product_id() != T3_DFU_PRODUCT_ID
            {
                return None;
            }

            let port_path = device.port_numbers().ok()?;
            let (access, name) = match device.open() {
                Ok(handle) => (DeviceAccess::Available, product_name(&handle, &descriptor)),
                Err(e) => {
                    let access = classify_open_error(e);
                    tracing::info!(
                        "T3 DFU device on bus {} is present but cannot be opened ({e}); listing it \
                         as {access:?}",
                        device.bus_number()
                    );
                    // The descriptor strings live behind the handle we could not get, so the
                    // identity is all there is to show.
                    (
                        access,
                        format!(
                            "T3 Gemstone board {:04x}:{:04x}",
                            descriptor.vendor_id(),
                            descriptor.product_id()
                        ),
                    )
                }
            };

            Some(Device {
                bus_num: device.bus_number(),
                port_num: device.port_number(),
                port_path,
                vendor_id: descriptor.vendor_id(),
                product_id: descriptor.product_id(),
                name,
                access,
            })
        })
        .collect()
}

/// Map an open failure onto the fix the user has to apply.
///
/// `Access` is the Linux permission case; `NotSupported`/`NotFound` is what libusb reports on
/// Windows when no WinUSB-compatible driver is bound. Anything else is reported as a driver problem
/// too, because from the screen's point of view "present but unopenable for an unknown reason" has
/// the same first remedy — and the exact chain is in the log either way.
const fn classify_open_error(error: rusb::Error) -> DeviceAccess {
    match error {
        rusb::Error::Access => DeviceAccess::PermissionDenied,
        _ => DeviceAccess::DriverMissing,
    }
}

fn product_name<C: rusb::UsbContext>(
    handle: &rusb::DeviceHandle<C>,
    descriptor: &rusb::DeviceDescriptor,
) -> String {
    const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

    let languages = handle.read_languages(TIMEOUT).unwrap_or_default();
    let language = languages.first().copied();
    let manufacturer = language.and_then(|lang| {
        handle
            .read_manufacturer_string(lang, descriptor, TIMEOUT)
            .ok()
    });
    let product =
        language.and_then(|lang| handle.read_product_string(lang, descriptor, TIMEOUT).ok());

    match (manufacturer, product) {
        (Some(manufacturer), Some(product)) => format!("{manufacturer}, {product}"),
        (_, Some(product)) => product,
        _ => format!(
            "USB device {:04x}:{:04x}",
            descriptor.vendor_id(),
            descriptor.product_id()
        ),
    }
}

/// Local-file parity harness retained for the CLI. It is deliberately strict: the caller must
/// provide exactly the three boot artifacts plus raw eMMC in contract order. Production callers
/// should obtain the first three inputs from [`BootArtifactResolver`], whose hashes are manifest
/// anchored. This harness spools and hashes each source before touching USB, avoiding the old
/// `u64 -> u32` conversion and ensuring a read error cannot begin a partial chain.
pub fn flash<R, I>(
    imgs: Vec<(String, R)>,
    vendor_id: u16,
    product_id: u16,
    bus_num: u8,
    port_num: u8,
    chan: Option<mpsc::SyncSender<DfuProgress>>,
    cancel: Option<CancellationToken>,
) -> Result<()>
where
    R: FnOnce() -> io::Result<(I, u64)>,
    I: io::Read,
{
    let expected_names = ["tiboot3.bin", "tispl.bin", "u-boot.img", "rawemmc"];
    if imgs.len() != expected_names.len()
        || imgs
            .iter()
            .zip(expected_names)
            .any(|((name, _), expected)| name != expected)
    {
        return Err(Error::InvalidPlan(format!(
            "local harness requires stages {:?} in this exact order",
            expected_names
        )));
    }

    let mut staged = Vec::with_capacity(4);
    for ((name, resolve), expected_name) in imgs.into_iter().zip(expected_names) {
        check_cancel(cancel.as_ref())?;
        let (mut reader, advertised_size) = resolve().map_err(Error::ResolverIo)?;
        if advertised_size == 0 {
            return Err(Error::EmptyImage(name));
        }
        let mut file = tempfile::tempfile().map_err(Error::ResolverIo)?;
        let mut hasher = Sha256::new();
        let mut copied = 0_u64;
        let mut buffer = vec![0_u8; 1024 * 1024];
        loop {
            check_cancel(cancel.as_ref())?;
            let read = reader
                .read(&mut buffer)
                .map_err(|source| Error::ImageRead {
                    artifact: name.clone(),
                    source,
                })?;
            if read == 0 {
                break;
            }
            file.write_all(&buffer[..read]).map_err(Error::ResolverIo)?;
            hasher.update(&buffer[..read]);
            copied += read as u64;
        }
        if copied != advertised_size {
            return Err(Error::ShortImage {
                artifact: name,
                expected: advertised_size,
                actual: copied,
            });
        }
        file.flush().map_err(Error::ResolverIo)?;
        file.rewind().map_err(Error::ResolverIo)?;
        let digest: [u8; 32] = hasher.finalize().into();
        let index = staged.len();
        let kind = if index < 3 {
            DfuStageKind::BootArtifact {
                next_alt_setting: if index < 2 {
                    expected_names[index + 1].to_owned()
                } else {
                    "rawemmc".to_owned()
                },
            }
        } else {
            DfuStageKind::RawEmmc
        };
        staged.push(DfuStageInput {
            stage: DfuStage {
                kind,
                artifact_name: expected_name.to_owned(),
                alt_setting: if index == 0 {
                    "bootloader".to_owned()
                } else {
                    expected_name.to_owned()
                },
                reset_after: index < 3,
                reconnect_timeout: std::time::Duration::from_secs(15),
                expected_sha256: digest,
                expected_size: Some(copied),
            },
            reader: Box::new(file),
        });
    }

    let mut transport = RusbTransport::new();
    let mut callback = |value: DfuProgress| {
        if let Some(sender) = &chan {
            let _ = sender.try_send(value);
        }
    };
    flash_with_transport(
        &mut transport,
        staged,
        vendor_id,
        product_id,
        UsbPath::legacy(bus_num, port_num),
        Some(&mut callback),
        cancel.as_ref(),
    )?;
    Ok(())
}

/// Read `reader` to its end, hashing every byte, optionally copying them into `sink`.
///
/// The progress it reports is what separates this pass from a hang: it walks the whole
/// multi-gigabyte image before the first USB packet, so a caller that stays silent here leaves the
/// front-end showing the previous phase for minutes.
fn digest_raw_image(
    reader: &mut dyn io::Read,
    mut sink: Option<&mut std::fs::File>,
    expected: u64,
    chan: Option<&mpsc::SyncSender<DfuProgress>>,
    cancel: Option<&CancellationToken>,
) -> Result<[u8; 32]> {
    let report = |fraction: f32| {
        if let Some(sender) = chan {
            let _ = sender.try_send(DfuProgress::ChecksummingImage(fraction));
        }
    };
    // Roughly 200 updates over the whole image, and never more than one per 8 MiB: the channel
    // drops on contention anyway, and the front-end only needs enough motion to show work.
    let step = (expected / 200).max(8 * 1024 * 1024);
    let mut next_report = step;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    let mut copied = 0_u64;

    report(0.0);
    loop {
        check_cancel(cancel)?;
        let read = reader
            .read(&mut buffer)
            .map_err(|source| Error::ImageRead {
                artifact: "rawemmc".to_owned(),
                source,
            })?;
        if read == 0 {
            break;
        }
        if let Some(file) = sink.as_mut() {
            file.write_all(&buffer[..read]).map_err(Error::ResolverIo)?;
        }
        hasher.update(&buffer[..read]);
        copied += read as u64;
        if copied >= next_report {
            next_report = copied + step;
            report(copied as f32 / expected as f32);
        }
    }

    if copied != expected {
        return Err(Error::ShortImage {
            artifact: "rawemmc".to_owned(),
            expected,
            actual: copied,
        });
    }
    report(1.0);
    Ok(hasher.finalize().into())
}

/// Spool a one-shot reader (typically a decompressor) into a scratch file so it can be hashed
/// before the write starts and replayed afterwards.
fn stage_raw_stream<R, I>(
    raw_image: R,
    chan: Option<&mpsc::SyncSender<DfuProgress>>,
    cancel: Option<&CancellationToken>,
) -> Result<DfuStageInput>
where
    R: FnOnce() -> io::Result<(I, u64)>,
    I: io::Read,
{
    let (mut reader, advertised_size) = raw_image().map_err(Error::ResolverIo)?;
    if advertised_size == 0 {
        return Err(Error::EmptyImage("rawemmc".to_owned()));
    }
    let mut file = tempfile::tempfile().map_err(Error::ResolverIo)?;
    let digest = digest_raw_image(&mut reader, Some(&mut file), advertised_size, chan, cancel)?;
    file.flush().map_err(Error::ResolverIo)?;
    file.rewind().map_err(Error::ResolverIo)?;
    Ok(raw_emmc_input(digest, advertised_size, Box::new(file)))
}

/// Hash an image that is already a finished file, in place.
///
/// The stream path has to spool because its reader is one-shot; a file is not, and copying it
/// would double both the wall-clock cost of this phase and the free space the whole operation
/// needs — for a multi-gigabyte image on a small system disk, that second copy is the difference
/// between working and failing.
fn stage_raw_file(
    path: &Path,
    chan: Option<&mpsc::SyncSender<DfuProgress>>,
    cancel: Option<&CancellationToken>,
) -> Result<DfuStageInput> {
    let mut file = std::fs::File::open(path).map_err(Error::ResolverIo)?;
    let size = file.metadata().map_err(Error::ResolverIo)?.len();
    if size == 0 {
        return Err(Error::EmptyImage("rawemmc".to_owned()));
    }
    let digest = digest_raw_image(&mut file, None, size, chan, cancel)?;
    file.rewind().map_err(Error::ResolverIo)?;
    Ok(raw_emmc_input(digest, size, Box::new(file)))
}

fn raw_emmc_input(digest: [u8; 32], size: u64, reader: Box<dyn io::Read + Send>) -> DfuStageInput {
    DfuStageInput {
        stage: DfuStage {
            kind: DfuStageKind::RawEmmc,
            artifact_name: "rawemmc".to_owned(),
            alt_setting: bb_config::t3::T3_RAW_EMMC_ALT_SETTING.to_owned(),
            reset_after: false,
            reconnect_timeout: bb_config::t3::T3_DFU_RECONNECT_TIMEOUT,
            expected_sha256: digest,
            expected_size: Some(size),
        },
        reader,
    }
}

/// Resolve the verified T3 boot chain and flash one extracted/customized raw eMMC image.
///
/// This is the production Phase 7 entry point. Unlike [`flash`], boot artifacts are never accepted
/// from arbitrary local name/path pairs: their names and hashes come from the strict boot manifest
/// and every object is re-hashed before reuse from cache.
pub fn flash_t3<R, I>(
    raw_image: R,
    path: UsbPath,
    cache_dir: impl AsRef<Path>,
    chan: Option<mpsc::SyncSender<DfuProgress>>,
    cancel: Option<CancellationToken>,
) -> Result<FlashReport>
where
    R: FnOnce() -> io::Result<(I, u64)>,
    I: io::Read,
{
    flash_t3_inner(
        |chan, cancel| stage_raw_stream(raw_image, chan, cancel),
        path,
        cache_dir,
        chan,
        cancel,
    )
}

/// [`flash_t3`] for an image that is already a file on disk — the GUI's staging file, which was
/// written and read-back verified before it got here.
pub fn flash_t3_file(
    raw_image: impl AsRef<Path>,
    path: UsbPath,
    cache_dir: impl AsRef<Path>,
    chan: Option<mpsc::SyncSender<DfuProgress>>,
    cancel: Option<CancellationToken>,
) -> Result<FlashReport> {
    let raw_image = raw_image.as_ref();
    flash_t3_inner(
        |chan, cancel| stage_raw_file(raw_image, chan, cancel),
        path,
        cache_dir,
        chan,
        cancel,
    )
}

fn flash_t3_inner(
    stage_raw: impl FnOnce(
        Option<&mpsc::SyncSender<DfuProgress>>,
        Option<&CancellationToken>,
    ) -> Result<DfuStageInput>,
    path: UsbPath,
    cache_dir: impl AsRef<Path>,
    chan: Option<mpsc::SyncSender<DfuProgress>>,
    cancel: Option<CancellationToken>,
) -> Result<FlashReport> {
    let cache_dir = cache_dir.as_ref();
    // Announced before the first network call: on a cold cache this phase can take a while, and a
    // screen that says nothing here looks like a hang before the board has even been touched.
    if let Some(sender) = &chan {
        let _ = sender.try_send(DfuProgress::BootArtifacts);
    }
    let profile = bb_config::t3::DfuProfile::t3_gem_o1();
    let downloader =
        bb_downloader::Downloader::new(cache_dir.join("objects")).map_err(Error::ResolverIo)?;
    let resolver = BootArtifactResolver::new(downloader, cache_dir.join("manifest.json"));
    let mut inputs = resolver
        .resolve_blocking(&profile, cancel.as_ref())?
        .into_iter()
        .map(ResolvedBootArtifact::into_input)
        .collect::<Result<Vec<_>>>()?;

    check_cancel(cancel.as_ref())?;
    inputs.push(stage_raw(chan.as_ref(), cancel.as_ref())?);

    let mut transport = RusbTransport::new();
    let mut callback = |value: DfuProgress| {
        if let Some(sender) = &chan {
            let _ = sender.try_send(value);
        }
    };
    flash_with_transport(
        &mut transport,
        inputs,
        profile.vendor_id,
        profile.product_id,
        path,
        Some(&mut callback),
        cancel.as_ref(),
    )
}

/// Platform-native persistent cache directory used by the CLI parity harness.
pub fn default_t3_cache_dir() -> Result<PathBuf> {
    directories::ProjectDirs::from("org", "t3gemstone", "T3GemstoneImager")
        .map(|dirs| dirs.cache_dir().join("dfu"))
        .ok_or_else(|| {
            Error::ResolverIo(io::Error::new(
                io::ErrorKind::NotFound,
                "the operating system did not provide an application cache directory",
            ))
        })
}

#[cfg(test)]
mod enumeration_tests {
    use super::*;

    /// The two failures a user can fix have to stay apart: a Linux permission problem is solved by
    /// a udev rule and a Windows driver problem by assigning WinUSB, and neither instruction helps
    /// with the other.
    #[test]
    fn open_failures_map_to_the_fix_the_user_has_to_apply() {
        assert_eq!(
            classify_open_error(rusb::Error::Access),
            DeviceAccess::PermissionDenied
        );
        assert_eq!(
            classify_open_error(rusb::Error::NotSupported),
            DeviceAccess::DriverMissing
        );
        // Unknown reasons take the driver branch rather than being dropped or reported as a
        // permission problem the user does not have.
        assert_eq!(
            classify_open_error(rusb::Error::Other),
            DeviceAccess::DriverMissing
        );
    }

    /// Enumeration must never invent a destination out of someone else's hardware. Without a board
    /// attached this is an emptiness check; with one attached it asserts the identity filter.
    #[test]
    fn only_the_t3_dfu_identity_is_ever_listed() {
        for device in devices(true).into_iter().chain(devices(false)) {
            assert_eq!(device.vendor_id, T3_DFU_VENDOR_ID);
            assert_eq!(device.product_id, T3_DFU_PRODUCT_ID);
        }
    }
}
