use std::{fmt, io, path::Path, time::Duration};

/// Stable USB topology identity. Device addresses and serial strings may change across reset;
/// bus plus the complete physical port chain does not as long as the cable stays in place.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UsbPath {
    pub bus: u8,
    pub ports: Vec<u8>,
}

impl UsbPath {
    pub fn new(bus: u8, ports: Vec<u8>) -> Result<Self, &'static str> {
        if ports.is_empty() {
            return Err("USB physical port path cannot be empty");
        }
        Ok(Self { bus, ports })
    }

    /// Compatibility selector for identifiers produced before full topology paths were exposed.
    pub fn legacy(bus: u8, port: u8) -> Self {
        Self {
            bus,
            ports: vec![port],
        }
    }

    pub fn matches(&self, other: &Self) -> bool {
        self.bus == other.bus
            && (self.ports == other.ports
                || (self.ports.len() == 1 && self.ports[0] == *other.ports.last().unwrap_or(&0)))
    }
}

impl fmt::Display for UsbPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02x}:", self.bus)?;
        for (index, port) in self.ports.iter().enumerate() {
            if index != 0 {
                f.write_str(".")?;
            }
            write!(f, "{port:02x}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DfuDevice {
    pub vendor_id: u16,
    pub product_id: u16,
    pub path: UsbPath,
    pub address: u8,
    pub serial: Option<String>,
    pub alt_settings: Vec<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DfuStageKind {
    BootArtifact { next_alt_setting: String },
    RawEmmc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DfuStage {
    pub kind: DfuStageKind,
    pub artifact_name: String,
    pub alt_setting: String,
    pub reset_after: bool,
    pub reconnect_timeout: Duration,
    pub expected_sha256: [u8; 32],
    pub expected_size: Option<u64>,
}

pub struct DfuStageInput {
    pub stage: DfuStage,
    pub reader: Box<dyn io::Read + Send>,
}

impl DfuStageInput {
    pub fn from_path(stage: DfuStage, path: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Self {
            stage,
            reader: Box::new(std::fs::File::open(path)?),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DfuTerminalEvidence {
    NextAltEnumerated(String),
    DfuIdle,
    ManifestWaitReset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DfuState {
    AppIdle = 0,
    AppDetach = 1,
    DfuIdle = 2,
    DfuDnloadSync = 3,
    DfuDnBusy = 4,
    DfuDnloadIdle = 5,
    DfuManifestSync = 6,
    DfuManifest = 7,
    DfuManifestWaitReset = 8,
    DfuUploadIdle = 9,
    DfuError = 10,
    Unknown = 255,
}

impl From<u8> for DfuState {
    fn from(value: u8) -> Self {
        match value {
            0 => Self::AppIdle,
            1 => Self::AppDetach,
            2 => Self::DfuIdle,
            3 => Self::DfuDnloadSync,
            4 => Self::DfuDnBusy,
            5 => Self::DfuDnloadIdle,
            6 => Self::DfuManifestSync,
            7 => Self::DfuManifest,
            8 => Self::DfuManifestWaitReset,
            9 => Self::DfuUploadIdle,
            10 => Self::DfuError,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DfuStatus {
    pub status: u8,
    pub poll_timeout: Duration,
    pub state: DfuState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportErrorKind {
    Disconnected,
    /// `LIBUSB_ERROR_NOT_FOUND`. On a control transfer this is a genuine "no such
    /// interface/alt-setting", but `libusb_reset_device` documents it as the *normal* answer when
    /// the device had to be re-enumerated — see [`Self::is_reset_reenumeration`].
    NotFound,
    Io,
    Timeout,
    Pipe,
    Access,
    Busy,
    InvalidParam,
    NoMem,
    NotSupported,
    Other,
}

impl TransportErrorKind {
    pub const fn may_be_reset_disconnect(self) -> bool {
        matches!(
            self,
            Self::Disconnected | Self::Io | Self::Timeout | Self::Pipe
        )
    }

    /// Whether this is how a *successful* device reset reports itself.
    ///
    /// Kept separate from [`Self::may_be_reset_disconnect`]: that predicate also guards the
    /// streaming path, where a missing interface must stay an error instead of being read as "the
    /// board booted".
    pub const fn is_reset_reenumeration(self) -> bool {
        matches!(self, Self::NotFound)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportError {
    pub kind: TransportErrorKind,
    pub message: String,
}

impl TransportError {
    pub fn new(kind: TransportErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for TransportError {}

/// What the DFU chain is doing right now.
///
/// A single 0..1 number cannot describe this chain honestly: it contains two phases whose duration
/// is not measurable in bytes (waiting for the board to re-enumerate, and the eMMC flush after the
/// last byte) and four transfers whose sizes differ by three orders of magnitude. Reporting each
/// phase by name lets the front-end weight them and show an indeterminate indicator exactly where
/// there is nothing to measure, instead of a bar that stalls and then jumps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DfuProgress {
    /// Resolving and verifying the three boot artifacts. Not byte-measured: they are small and
    /// usually served from cache.
    BootArtifacts,
    /// Reading the raw eMMC image end to end to establish the digest the write is verified against.
    ///
    /// Byte-measured on purpose: this pass touches the whole multi-gigabyte image before a single
    /// USB packet is sent, and a screen that keeps saying "verifying boot files" through all of it
    /// is indistinguishable from a hang.
    ChecksummingImage(f32),
    /// Waiting for the board to disappear and come back on the same physical port.
    Reconnecting,
    /// Transferring boot stage `index` (1-based, of three); `fraction` is within that stage.
    BootStage { index: u8, fraction: f32 },
    /// Streaming the raw eMMC image — the dominant cost of the whole operation.
    RawWrite(f32),
    /// After the last byte: manifest, flush and final detach. The board is writing, so this takes
    /// minutes with nothing to count.
    Finalizing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageReport {
    pub artifact_name: String,
    pub bytes_sent: u64,
    pub terminal_evidence: DfuTerminalEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlashReport {
    pub stages: Vec<StageReport>,
}
