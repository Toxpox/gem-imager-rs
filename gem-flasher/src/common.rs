use std::borrow::Cow;

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum DownloadFlashingStatus {
    Preparing,
    DownloadingProgress(f32),
    FlashingProgress(f32),
    Verifying(f32),
    Customizing,

    ResolvingBootArtifacts,
    ChecksummingImage(f32),
    Reconnecting,
    BootStage {
        stage: u8,
        progress: f32,
    },
    RawWrite(f32),
    Finalizing,
}

pub trait GemFlasherTarget
where
    Self: Sized,
{
    const FILE_TYPES: &[&str];
    const IS_DESTINATION_SELECTABLE: bool = true;

    fn destinations(filter: bool) -> Vec<Self>;

    fn identifier<'a>(&'a self) -> Cow<'a, str>;
}
