
mod common;
mod flasher;
pub mod img;
#[cfg(feature = "t3_gem_init")]
pub mod t3_gem_init;

use std::path::Path;

pub use common::*;
#[allow(unused_imports)]
pub use flasher::*;

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct LocalImage(Box<Path>);

impl LocalImage {
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
            let img = img::OsImage::from_path(&self.0, img::ExtractGate::LocalFile)?;
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
