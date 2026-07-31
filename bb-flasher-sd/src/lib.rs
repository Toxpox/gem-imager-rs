//! Library to flash SD cards with OS images. Powers sd card flashing in [BeagleBoard Imager].
//!
//! Also allows optional extra [Customization] for BeagleBoard images.
//!
//! # Platform Support
//!
//! - Linux
//! - Windows
//! - MacOS
//!
//! # Features
//!
//! - `udev`: Dynamic permissions on Linux. Mostly useful for GUI and flatpaks
//! - `macos_authopen`: Dynamic permissions on MacOS.
//!
//! [BeagleBoard Imager]: https://github.com/beagleboard/bb-imager-rs

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
/// Errors for this crate
pub enum Error {
    /// The partition table of image invalid.
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
    /// Unknown error occured during IO.
    #[error("Unknown Error during IO. Please check logs for more information.")]
    IoError {
        #[from]
        #[source]
        source: io::Error,
    },
    /// Aborted before completing
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

    /// The image stream ended before the declared size was written.
    ///
    /// Silently succeeding here produces a card that flashes "successfully" and then fails to
    /// boot, which is the hardest failure for a user to attribute.
    #[error("Only {written} of {expected} bytes reached the destination.")]
    ShortWrite { expected: u64, written: u64 },

    /// The device gave back something other than what was written to it.
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
        "Destination is too small: the image needs {required} bytes but the device has only {available}."
    )]
    InsufficientCapacity { required: u64, available: u64 },

    #[error("Refusing to write to \"{name}\": it is reported as a system disk.")]
    SystemDisk { name: Box<str> },

    /// Buffers could not be made durable. Never downgraded to a warning: an unsynced tail is
    /// indistinguishable from a successful flash until the board fails to boot.
    #[error("Failed to flush written data to the destination.")]
    SyncFailed {
        #[source]
        source: io::Error,
    },

    #[cfg(windows)]
    #[error("Failed to clear SD Card.")]
    WindowsCleanError(std::process::Output),
}

/// Enumerate all SD Cards in system
pub fn devices(filter: bool) -> Vec<Device> {
    bb_drivelist::drive_list()
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
            Device::new(
                x.description,
                x.raw.into(),
                x.size.unwrap_or_default(),
                x.is_system,
            )
        })
        .collect()
}

#[derive(Hash, Debug, PartialEq, Eq, Clone)]
/// SD Card
pub struct Device {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    /// Whether the platform reports this as a disk the running system depends on.
    ///
    /// Carried on the device rather than filtered away at enumeration so callers that list
    /// unfiltered destinations can still show it and refuse it, instead of it quietly reappearing
    /// as a selectable target.
    pub is_system: bool,
}

impl Device {
    const fn new(name: String, path: PathBuf, size: u64, is_system: bool) -> Self {
        Self {
            name,
            path,
            size,
            is_system,
        }
    }
}

/// Format SD card to fat32
pub fn format(dst: &std::path::Path) -> Result<()> {
    crate::pal::format(dst)
}

#[derive(Debug, Clone)]
pub enum Destination {
    File(Box<Path>),
    SdCard(Box<Path>),
}
