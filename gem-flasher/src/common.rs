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

    // ---- DFU / eMMC phases -------------------------------------------------------------------
    // The DFU chain is not one write. Collapsing it into `FlashingProgress` would mean either a
    // bar that restarts four times or one whose first 3 % of bytes occupy most of the motion, and
    // it would have to invent a number for the two phases that cannot be measured at all.
    /// Resolving and verifying the three boot artifacts. Indeterminate.
    ResolvingBootArtifacts,
    /// Hashing the staged image so the eMMC write can be verified against it. Byte-measured: it
    /// reads the whole image before the first USB packet and would otherwise look like a stall.
    ChecksummingImage(f32),
    /// Waiting for the board to re-enumerate after a reset. Indeterminate.
    Reconnecting,
    /// Transferring boot stage `stage` of three; the fraction is within that stage.
    BootStage {
        stage: u8,
        progress: f32,
    },
    /// Streaming the raw image to onboard eMMC — the dominant cost.
    RawWrite(f32),
    /// Manifest, flush and final detach after the last byte. Indeterminate, and minutes long.
    Finalizing,
}

/// A trait for modeling flasher targets.
///
/// Some flashers have a single target (for example a subprocessor in SBC).
pub trait GemFlasherTarget
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
