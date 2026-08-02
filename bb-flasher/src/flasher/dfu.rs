use crate::common::{BBFlasherTarget, DownloadFlashingStatus};

use std::borrow::Cow;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc;

use bb_helper::cancel::CancellationToken;

/// Re-exported so front-ends can render the reason a listed board is unusable without depending on
/// the backend crate directly.
pub use bb_flasher_dfu::DeviceAccess;

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct Target(bb_flasher_dfu::Device);

impl Target {
    fn destinations_internal(filter: bool) -> Vec<Self> {
        bb_flasher_dfu::devices(filter)
            .into_iter()
            .map(Self)
            .collect()
    }

    pub const fn bus_number(&self) -> u8 {
        self.0.bus_num
    }

    pub const fn port_num(&self) -> u8 {
        self.0.port_num
    }

    pub fn port_path(&self) -> &[u8] {
        &self.0.port_path
    }

    pub const fn vendor_id(&self) -> u16 {
        self.0.vendor_id
    }

    pub const fn product_id(&self) -> u16 {
        self.0.product_id
    }

    /// Whether this board can be opened, and if not, what the user has to fix.
    ///
    /// A board that cannot be opened is still offered as a destination; the front-end turns this
    /// into the WinUSB or udev instruction instead of silently hiding the hardware.
    pub const fn access(&self) -> bb_flasher_dfu::DeviceAccess {
        self.0.access
    }
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(f, "{}", self.0.name)
    }
}

impl BBFlasherTarget for Target {
    const FILE_TYPES: &[&str] = &[];

    /// The flag is accepted for the trait's sake and deliberately ignored: see
    /// [`bb_flasher_dfu::devices`]. There is no hidden-but-valid DFU device to reveal, only other
    /// vendors' hardware to mis-offer.
    fn destinations(filter: bool) -> Vec<Self> {
        Self::destinations_internal(filter)
    }

    fn identifier(&self) -> Cow<'_, str> {
        let ports = self
            .0
            .port_path
            .iter()
            .map(|port| format!("{port:02x}"))
            .collect::<Vec<_>>()
            .join(".");
        Cow::Owned(format!(
            "{:02x}:{ports}:{:04x}:{:04x}",
            self.0.bus_num, self.0.vendor_id, self.0.product_id
        ))
    }
}

pub struct Flasher<R> {
    raw_image: R,
    path: bb_flasher_dfu::UsbPath,
    cache_dir: PathBuf,
    cancel: Option<CancellationToken>,
}

impl<R> Flasher<R> {
    pub fn from_identifier(
        raw_image: R,
        id: &str,
        cancel: Option<CancellationToken>,
    ) -> io::Result<Self> {
        let ids = id.split(':').map(str::trim).collect::<Vec<_>>();
        if ids.len() != 4 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "identifier must be bus:physical-port-path:vendor:product",
            ));
        }
        let bus = u8::from_str_radix(ids[0], 16)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid bus number"))?;
        let ports = ids[1]
            .split('.')
            .map(|component| {
                u8::from_str_radix(component, 16).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidInput, "invalid physical port path")
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        let vendor_id = u16::from_str_radix(ids[2], 16)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid vendor ID"))?;
        let product_id = u16::from_str_radix(ids[3], 16)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid product ID"))?;
        if vendor_id != bb_flasher_dfu::T3_DFU_VENDOR_ID
            || product_id != bb_flasher_dfu::T3_DFU_PRODUCT_ID
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "T3 DFU requires {:04x}:{:04x}",
                    bb_flasher_dfu::T3_DFU_VENDOR_ID,
                    bb_flasher_dfu::T3_DFU_PRODUCT_ID
                ),
            ));
        }
        let path = bb_flasher_dfu::UsbPath::new(bus, ports)
            .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?;
        let cache_dir = bb_flasher_dfu::default_t3_cache_dir().map_err(io::Error::other)?;
        Ok(Self {
            raw_image,
            path,
            cache_dir,
            cancel,
        })
    }

    pub fn with_cache_dir(mut self, cache_dir: impl Into<PathBuf>) -> Self {
        self.cache_dir = cache_dir.into();
        self
    }
}

impl<R> Flasher<R>
where
    R: FnOnce() -> std::io::Result<(crate::img::OsImage, u64)>,
{
    pub fn flash(
        self,
        chan: Option<mpsc::SyncSender<DownloadFlashingStatus>>,
    ) -> anyhow::Result<()> {
        std::thread::scope(|scope| {
            let progress = if let Some(chan) = chan {
                let (tx, rx) = mpsc::sync_channel(2);
                scope.spawn(move || {
                    while let Ok(value) = rx.recv() {
                        let _ = chan.try_send(translate_progress(value));
                    }
                });
                Some(tx)
            } else {
                None
            };

            bb_flasher_dfu::flash_t3(
                self.raw_image,
                self.path,
                self.cache_dir,
                progress,
                self.cancel,
            )
            .map(|_| ())
            .map_err(Into::into)
        })
    }
}

/// Map a backend phase onto the shared front-end status.
///
/// One-to-one on purpose: the backend already decided which phases are measurable, and re-deriving
/// that here is how the two ends drift apart.
const fn translate_progress(value: bb_flasher_dfu::DfuProgress) -> DownloadFlashingStatus {
    match value {
        bb_flasher_dfu::DfuProgress::BootArtifacts => {
            DownloadFlashingStatus::ResolvingBootArtifacts
        }
        bb_flasher_dfu::DfuProgress::Reconnecting => DownloadFlashingStatus::Reconnecting,
        bb_flasher_dfu::DfuProgress::BootStage { index, fraction } => {
            DownloadFlashingStatus::BootStage {
                stage: index,
                progress: fraction,
            }
        }
        bb_flasher_dfu::DfuProgress::RawWrite(x) => DownloadFlashingStatus::RawWrite(x),
        bb_flasher_dfu::DfuProgress::Finalizing => DownloadFlashingStatus::Finalizing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every backend phase must survive as its own front-end phase; a translation that collapsed
    /// two of them would put the screen back to guessing.
    #[test]
    fn each_backend_phase_keeps_its_identity() {
        assert_eq!(
            translate_progress(bb_flasher_dfu::DfuProgress::BootArtifacts),
            DownloadFlashingStatus::ResolvingBootArtifacts
        );
        assert_eq!(
            translate_progress(bb_flasher_dfu::DfuProgress::Reconnecting),
            DownloadFlashingStatus::Reconnecting
        );
        assert_eq!(
            translate_progress(bb_flasher_dfu::DfuProgress::BootStage {
                index: 2,
                fraction: 0.5
            }),
            DownloadFlashingStatus::BootStage {
                stage: 2,
                progress: 0.5
            }
        );
        assert_eq!(
            translate_progress(bb_flasher_dfu::DfuProgress::RawWrite(0.25)),
            DownloadFlashingStatus::RawWrite(0.25)
        );
        assert_eq!(
            translate_progress(bb_flasher_dfu::DfuProgress::Finalizing),
            DownloadFlashingStatus::Finalizing
        );
    }

    #[test]
    fn parses_full_physical_port_path_and_rejects_non_t3_ids() {
        let image = || -> io::Result<(crate::img::OsImage, u64)> { unreachable!() };
        let flasher = Flasher::from_identifier(image, "03:02.07:0451:6165", None).unwrap();
        assert_eq!(flasher.path.bus, 3);
        assert_eq!(flasher.path.ports, [2, 7]);

        let image = || -> io::Result<(crate::img::OsImage, u64)> { unreachable!() };
        assert!(Flasher::from_identifier(image, "03:07:1234:5678", None).is_err());
    }
}
