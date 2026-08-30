use std::{
    io,
    path::{Path, PathBuf},
};

use thiserror::Error;

pub(crate) mod customization;
mod flashing;
mod helpers;
#[cfg(any(feature = "mock_sd", test))]
pub mod mock_sd;
pub(crate) mod pal;

pub use customization::{ContentType, Customization, ParitionType};
pub use flashing::{Status, flash};

pub(crate) type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Partition table of image not valid.")]
    InvalidPartitionTable,
    #[error("Only FAT BOOT partitions are supported.")]
    InvalidBootPartition,
    #[error("Failed to create customization {file}")]
    CustomizationFileCreateFail {
        #[source]
        source: io::Error,
        file: Box<str>,
    },

    #[error(
        "Read-back verification failed for {file}: the boot partition does not hold the bytes \
         that were written. The card may be faulty, counterfeit, or was disconnected."
    )]
    CustomizationReadBackMismatch { file: Box<str> },
    #[error("Unknown Error during IO. Please check logs for more information.")]
    IoError {
        #[from]
        #[source]
        source: io::Error,
    },
    #[error("Aborted before completing.")]
    Aborted,
    #[error("Failed to format SD Card.")]
    FailedToFormat {
        #[source]
        source: io::Error,
    },
    #[error("Failed to open SD Card.")]
    FailedToOpenDestination {
        #[source]
        source: anyhow::Error,
    },
    #[error("Writer thread has been closed.")]
    WriterClosed,

    #[error("Only {written} of {expected} bytes reached the destination.")]
    ShortWrite { expected: u64, written: u64 },

    #[error(
        "Read-back verification failed: the destination holds different data than was written \
         (expected sha256 {expected}, device returned {actual}). The card may be faulty, \
         counterfeit, or was disconnected during writing."
    )]
    ReadBackMismatch {
        expected: Box<str>,
        actual: Box<str>,
    },

    #[error(
        "Read-back verification failed for the published partition layout. The card may be faulty, counterfeit, or was disconnected during writing."
    )]
    LayoutReadBackMismatch,

    #[error(
        "Destination is too small: the image needs {required} bytes but the device has only {available}."
    )]
    InsufficientCapacity { required: u64, available: u64 },

    #[error("Refusing to write to \"{name}\": it is reported as a system disk.")]
    SystemDisk { name: Box<str> },

    #[error(
        "Refusing to write to \"{name}\": a different device now occupies this path. Reselect the \
         destination and try again."
    )]
    DestinationChanged { name: Box<str> },

    #[error(
        "Refusing to write to \"{path}\": it is not a recognised removable device. Reconnect the \
         card and try again."
    )]
    UnknownDestination { path: Box<str> },

    #[error("Failed to flush written data to the destination.")]
    SyncFailed {
        #[source]
        source: io::Error,
    },

    #[cfg(windows)]
    #[error("Failed to clear SD Card.")]
    WindowsCleanError(std::process::Output),

    #[cfg(windows)]
    #[error("Failed to enumerate Windows volumes.")]
    WindowsVolumeEnumeration {
        #[source]
        source: io::Error,
    },

    #[cfg(windows)]
    #[error("Failed to lock Windows volume {volume} for raw customization writes.")]
    WindowsVolumeLock {
        #[source]
        source: io::Error,
        volume: Box<str>,
    },

    #[cfg(windows)]
    #[error(
        "Volume {volume} spans physical disk {disk_number} and another disk; refusing to lock it."
    )]
    WindowsSpannedVolume { volume: Box<str>, disk_number: u32 },
}

pub fn devices(filter: bool) -> Vec<Device> {
    gem_drivelist::drive_list()
        .expect("Unsupported OS for Sd Card")
        .into_iter()
        .filter(|x| {
            if filter {
                x.is_removable && !x.is_virtual
            } else {
                true
            }
        })
        .map(|x| {
            let identity = DeviceIdentity {
                serial: x.serial,
                wwn: x.wwn,
                size: x.size.unwrap_or_default(),
            };

            Device::new(
                x.description,
                x.raw.into(),
                x.size.unwrap_or_default(),
                x.is_system,
                identity,
            )
        })
        .collect()
}

#[derive(Hash, Debug, PartialEq, Eq, Clone)]
pub struct Device {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub is_system: bool,
    pub identity: DeviceIdentity,
}

impl Device {
    const fn new(
        name: String,
        path: PathBuf,
        size: u64,
        is_system: bool,
        identity: DeviceIdentity,
    ) -> Self {
        Self {
            name,
            path,
            size,
            is_system,
            identity,
        }
    }
}

#[derive(Hash, Debug, PartialEq, Eq, Clone, Default)]
pub struct DeviceIdentity {
    pub serial: Option<String>,
    pub wwn: Option<String>,
    pub size: u64,
}

impl DeviceIdentity {
    pub fn is_distinguishing(&self) -> bool {
        self.serial.is_some() || self.wwn.is_some()
    }

    pub fn matches(&self, other: &Self) -> bool {
        if self.size != other.size {
            return false;
        }

        self.serial == other.serial && self.wwn == other.wwn
    }
}

pub fn format(dst: &std::path::Path, identity: &DeviceIdentity) -> Result<()> {
    crate::pal::format(dst, identity)
}

#[derive(Debug, Clone)]
pub enum Destination {
    File(Box<Path>),
    SdCard(Box<Path>, DeviceIdentity),
}
