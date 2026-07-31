//! Stuff common to all the flashers

use std::borrow::Cow;

/// Enum to denote the Flashing progress.
///
/// The progress is denoted by [f32] between 0 and 1
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum DownloadFlashingStatus {
    Preparing,
    DownloadingProgress(f32),
    FlashingProgress(f32),
    /// Reading the written data back off the destination and comparing it.
    ///
    /// Carries its own fraction rather than being a bare marker: a full read-back takes about as
    /// long as the write it verifies, and a UI that shows no movement for minutes reads as a hang.
    Verifying(f32),
    Customizing,
}

/// A trait for modeling flasher targets.
///
/// Some flashers have a single target (for example a subprocessor in SBC).
pub trait BBFlasherTarget
where
    Self: Sized,
{
    /// File types (extensions) supported by the flasher. Can be used for filtering local files in
    /// applications
    const FILE_TYPES: &[&str];
    const IS_DESTINATION_SELECTABLE: bool = true;

    /// A list of possible flasher targets
    fn destinations(filter: bool) -> Vec<Self>;

    /// A sort of device ID (mostly a Path).
    fn identifier<'a>(&'a self) -> Cow<'a, str>;
}
