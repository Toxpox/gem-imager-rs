//! # Introduction
//!
//! This crate provides common abstractions over the different flashers to be used by applications
//! such as BeagleBoard Imaging Utility. It also provides traits to add more flashers which behave
//! similiar to the pre-defined ones
//!
//! # Usage
//!
//! ```no_run
//! use std::path::PathBuf;
//!
//! let img = bb_flasher::LocalImage::new(PathBuf::from("/tmp/abc.img.xz").into());
//! let target = PathBuf::from("/tmp/target").try_into().unwrap();
//! let customization =
//!     bb_flasher::sd::FlashingSdLinuxConfig::sysconfig(None, None, None, None, None, None, None);
//!
//! bb_flasher::sd::Flasher::new(img.into_image_fn(), target, customization)
//!     .flash(None, None)
//!     .unwrap();
//! ```
//!
//! # Features
//!
//! - `sd`: Provide flashing Linux images to SD Cards. Enabled by **default**.
//! - `sd_linux_udev`: Uses udev to provide GUI prompt to open SD Cards in Linux. Useful for GUI
//!   applications.
//! - `sd_macos_authopen`: Uses authopen to provide GUI prompt to open SD Cards in MacOS. Useful
//!   for GUI applications.
//! - `dfu`: Provide USB DFU flashing.

mod common;
mod flasher;
pub mod img;

use std::path::Path;

pub use common::*;
#[allow(unused_imports)]
pub use flasher::*;

/// An Os Image present in the local filesystem
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct LocalImage(Box<Path>);

impl LocalImage {
    /// Construct a new local image from path.
    pub const fn new(path: Box<Path>) -> Self {
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn file_name(&self) -> &std::ffi::OsStr {
        self.0.file_name().unwrap()
    }

    pub fn into_image_fn(self) -> impl FnOnce() -> std::io::Result<(img::OsImage, u64)> {
        move || {
            let img = img::OsImage::from_path(&self.0)?;
            let size = img.size();

            Ok((img, size))
        }
    }
}

impl std::fmt::Display for LocalImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            self.0
                .file_name()
                .expect("image cannot be a directory")
                .to_string_lossy()
        )
    }
}
