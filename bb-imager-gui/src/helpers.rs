use std::io;
use std::{borrow::Cow, fmt::Display, path::PathBuf, sync::LazyLock, time::Duration};

use crate::{BBImagerMessage, PACKAGE_QUALIFIER, constants};
use bb_config::config;
use bb_flasher::img::OsImage;
use bb_flasher::{BBFlasherTarget, DownloadFlashingStatus};
use bb_helper::file_stream::ReaderFileStream;
use std::sync::mpsc;
use tokio_util::task::AbortOnDropHandle;
use url::Url;

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) enum BoardImageIcon {
    Remote(url::Url),
    Local,
    Format,
}

#[derive(Debug, Clone, serde::Serialize)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum BoardImage {
    SdFormat {
        details: Vec<(&'static str, String)>,
    },
    Image {
        flasher: config::Flasher,
        init_format: config::InitFormat,
        img: SelectedImage,
        info_text: Option<String>,
        description: Option<String>,
        icon: BoardImageIcon,
        details: Vec<(&'static str, String)>,
        support: Option<Url>,
    },
}

impl BoardImage {
    pub(crate) fn local(path: PathBuf, flasher: config::Flasher) -> Self {
        let metadata = std::fs::metadata(&path).expect("File does not exist");
        let details = vec![
            ("Path", path.to_string_lossy().to_string()),
            ("Size", metadata.len().to_string()),
        ];

        Self::Image {
            img: bb_flasher::LocalImage::new(path.into()).into(),
            flasher,
            // Do not try to apply customization for local images
            init_format: config::InitFormat::None,
            info_text: None,
            description: None,
            icon: BoardImageIcon::Local,
            details,
            support: None,
        }
    }

    pub(crate) fn remote(
        image: crate::db::OsImage,
        flasher: config::Flasher,
        downloader: bb_downloader::Downloader,
    ) -> Self {
        let mut details = vec![
            ("Release Date", image.release_date.to_string()),
            ("Image Size", pretty_bytes(image.extract_size as u64)),
        ];

        if let Some(x) = image.image_download_size {
            details.push(("Download Size", pretty_bytes(x as u64)))
        }

        Self::Image {
            img: RemoteImage::new(
                image.name.into(),
                Box::new(image.url),
                image.image_download_sha256,
                image.image_download_size.map(|x| x as u64),
                image.extract_sha256,
                image.extract_size as u64,
                downloader.clone(),
            )
            .into(),
            flasher,
            init_format: image.init_format,
            info_text: image.info_text,
            description: Some(image.description),
            icon: BoardImageIcon::Remote(image.icon),
            details,
            support: image.support,
        }
    }

    pub(crate) fn format() -> Self {
        Self::SdFormat {
            details: vec![("Format", "FAT32".to_string())],
        }
    }

    pub(crate) fn description(&self) -> Option<&str> {
        match self {
            BoardImage::SdFormat { .. } => Some("Format a SD Card to FAT32 for reuse."),
            BoardImage::Image { description, .. } => description.as_ref().map(|x| x.as_str()),
        }
    }

    pub(crate) fn icon(&self) -> &BoardImageIcon {
        match self {
            BoardImage::SdFormat { .. } => &BoardImageIcon::Format,
            BoardImage::Image { icon, .. } => icon,
        }
    }

    pub(crate) const fn flasher(&self) -> config::Flasher {
        match self {
            BoardImage::SdFormat { .. } => config::Flasher::SdCard,
            BoardImage::Image { flasher, .. } => *flasher,
        }
    }

    pub(crate) const fn init_format(&self) -> config::InitFormat {
        match self {
            BoardImage::Image { init_format, .. } => *init_format,
            BoardImage::SdFormat { .. } => config::InitFormat::None,
        }
    }

    pub(crate) fn info_text(&self) -> Option<&str> {
        match self {
            BoardImage::Image { info_text, .. } => info_text.as_ref().map(|x| x.as_str()),
            BoardImage::SdFormat { .. } => None,
        }
    }

    pub(crate) fn file_name(&self) -> Option<String> {
        match self {
            Self::SdFormat { .. } => None,
            Self::Image { img, .. } => Some(img.file_name()),
        }
    }

    pub(crate) fn details(&self) -> &[(&'static str, String)] {
        match self {
            BoardImage::SdFormat { details } => details,
            BoardImage::Image { details, .. } => details,
        }
    }

    pub(crate) fn supported_init_formats(&self) -> &'static [config::InitFormat] {
        match self {
            BoardImage::SdFormat { .. } => &[],
            BoardImage::Image {
                img,
                init_format,
                flasher,
                ..
            } if !matches!(img, SelectedImage::LocalImage(_)) => match init_format {
                config::InitFormat::Sysconf => &[config::InitFormat::Sysconf],
                config::InitFormat::CloudInit => &[config::InitFormat::CloudInit],
                _ => &[],
            },
            BoardImage::Image {
                init_format,
                flasher,
                ..
            } if *flasher == config::Flasher::SdCard => {
                &[config::InitFormat::Sysconf, config::InitFormat::CloudInit]
            }
            BoardImage::Image { .. } => &[],
        }
    }

    pub(crate) fn update_init_format(&mut self, f: config::InitFormat) {
        match self {
            BoardImage::SdFormat { .. } => {
                unreachable!();
            }
            BoardImage::Image { init_format, .. } => {
                *init_format = f;
            }
        }
    }

    pub(crate) fn support(&self) -> Option<&Url> {
        match self {
            BoardImage::SdFormat { .. } => None,
            BoardImage::Image { support, .. } => support.as_ref(),
        }
    }
}

impl std::fmt::Display for BoardImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BoardImage::SdFormat { .. } => write!(f, "Format SD Card"),
            BoardImage::Image { img: image, .. } => image.fmt(f),
        }
    }
}

pub(crate) fn system_timezone() -> Option<chrono_tz::Tz> {
    static SYSTEM_TIMEZONE: LazyLock<Option<chrono_tz::Tz>> =
        LazyLock::new(|| iana_time_zone::get_timezone().ok()?.parse().ok());
    *SYSTEM_TIMEZONE
}

pub(crate) fn system_keymap() -> &'static str {
    static SYSTEM_KEYMAP: LazyLock<Option<&'static str>> = LazyLock::new(|| {
        let lang = whoami::lang_prefs().ok()?.message_langs().next()?;
        let lang_str = lang.to_string();

        let base = lang_str.split('.').next().unwrap_or(&lang_str);
        let mut parts = base.split(['-', '_', '/']);

        parts.next();
        if let Some(region) = parts.next() {
            let region = region.split('@').next().unwrap_or(region).trim();
            if !region.is_empty()
                && let Some(&canon) = crate::constants::KEYMAP_LAYOUTS
                    .iter()
                    .find(|k| k.eq_ignore_ascii_case(region))
            {
                return Some(canon);
            }
        }

        None
    });
    (*SYSTEM_KEYMAP).unwrap_or("us")
}

/// A catalog image, with the integrity values the catalog published for it.
///
/// The two hashes are *not* interchangeable and never share a name (`instruction.md` §6.2):
/// `archive_sha256` covers the compressed download and is what the cache is addressed by;
/// `extract_sha256` covers the bytes that reach the board. Before Faz 3 this struct called the
/// archive hash `extract_sha256` while being handed `image_download_sha256`, so the extracted
/// bytes were never checked at all.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct RemoteImage {
    name: Box<str>,
    url: Box<url::Url>,
    #[serde(with = "const_hex")]
    archive_sha256: [u8; 32],
    archive_size: Option<u64>,
    /// `None` when the catalog entry predates the extracted-digest contract.
    #[serde(skip)]
    extract_sha256: Option<[u8; 32]>,
    extract_size: u64,
    #[serde(skip)]
    downloader: bb_downloader::Downloader,
}

impl RemoteImage {
    pub(crate) fn new(
        name: Box<str>,
        url: Box<url::Url>,
        archive_sha256: [u8; 32],
        archive_size: Option<u64>,
        extract_sha256: Option<[u8; 32]>,
        extract_size: u64,
        downloader: bb_downloader::Downloader,
    ) -> Self {
        Self {
            name,
            url,
            archive_sha256,
            archive_size,
            extract_sha256,
            extract_size,
            downloader,
        }
    }

    /// The extracted-side gate for this image.
    ///
    /// A catalog entry that publishes an extracted digest is held to it; one that does not is
    /// marked as such rather than quietly treated as verified.
    fn extract_gate(&self) -> bb_flasher::img::ExtractGate {
        match self.extract_sha256 {
            Some(sha256) => bb_flasher::img::ExtractGate::Declared(
                bb_flasher::img::ExtractedIntegrity::new(self.extract_size, sha256),
            ),
            None => bb_flasher::img::ExtractGate::UndeclaredLegacyCatalog,
        }
    }

    fn file_name(&self) -> &str {
        self.url.path_segments().unwrap().next_back().unwrap()
    }

    fn open<C, P, R>(self, f_cache: C, f_pipe: P) -> impl FnOnce() -> io::Result<R>
    where
        C: FnOnce(&std::path::Path) -> io::Result<R>,
        P: FnOnce(ReaderFileStream, AbortOnDropHandle<io::Result<()>>, u64) -> io::Result<R>,
    {
        let rt = tokio::runtime::Handle::current();
        move || {
            let downloader = self.downloader.clone();
            // The cache is addressed by the *archive* hash, because the archive is what is stored.
            let cache = downloader.check_cache_from_sha(self.archive_sha256);

            if let Some(path) = cache {
                tracing::info!("Found the remote image in cache");
                return f_cache(&path);
            }

            tracing::info!("Remote image not found in cache. Downloading");
            let (tx_stream, rx) = bb_helper::file_stream::file_stream()?;
            let downloader = self.downloader.clone();
            let url = self.url.clone();
            let integrity = bb_downloader::ArchiveIntegrity {
                sha256: self.archive_sha256,
                size: self.archive_size,
            };

            let t: tokio::task::JoinHandle<io::Result<()>> = rt.spawn(async move {
                downloader
                    .download_to_stream(*url, integrity, tx_stream)
                    .await
                    .map_err(|e| {
                        let msg = format!("Error while downloading Os Image: {e}");
                        tracing::error!("{}", &msg);
                        io::Error::other(msg)
                    })?;
                tracing::info!("Image download finished");
                Ok(())
            });

            f_pipe(rx, AbortOnDropHandle::new(t), self.extract_size)
        }
    }

    fn into_image_fn(self) -> impl FnOnce() -> io::Result<(OsImage, u64)> {
        let extract_size = self.extract_size;
        // Captured before `self` is consumed; the gate is the same whether the archive comes from
        // the cache or straight off the wire.
        let gate = self.extract_gate();
        self.open(
            move |p| Ok((OsImage::from_path(p, gate)?, extract_size)),
            move |rx, abort, es| {
                let img = OsImage::from_piped(rx, abort, es, gate)?;
                Ok((img, es))
            },
        )
    }
}

impl std::fmt::Display for RemoteImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) enum SelectedImage {
    LocalImage(bb_flasher::LocalImage),
    /// Boxed because a remote image carries the whole published integrity set while a local one is
    /// just a path; inlining it would make every `SelectedImage` pay for the larger variant.
    RemoteImage(Box<RemoteImage>),
}

impl SelectedImage {
    fn file_name(&self) -> String {
        match self {
            Self::LocalImage(x) => x.file_name().to_string_lossy().to_string(),
            Self::RemoteImage(x) => x.file_name().to_string(),
        }
    }

    fn into_image_fn(self) -> Box<dyn FnOnce() -> io::Result<(OsImage, u64)> + Send> {
        match self {
            SelectedImage::LocalImage(x) => Box::new(x.into_image_fn()),
            SelectedImage::RemoteImage(x) => Box::new((*x).into_image_fn()),
        }
    }
}

impl std::fmt::Display for SelectedImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SelectedImage::LocalImage(x) => x.fmt(f),
            SelectedImage::RemoteImage(x) => x.fmt(f),
        }
    }
}

impl From<RemoteImage> for SelectedImage {
    fn from(value: RemoteImage) -> Self {
        Self::RemoteImage(Box::new(value))
    }
}

impl From<bb_flasher::LocalImage> for SelectedImage {
    fn from(value: bb_flasher::LocalImage) -> Self {
        Self::LocalImage(value)
    }
}

pub(crate) async fn flash(
    img: BoardImage,
    customization: FlashingCustomization,
    dst: Destination,
    chan: mpsc::SyncSender<DownloadFlashingStatus>,
    cancel_sync: bb_helper::cancel::CancellationToken,
) -> anyhow::Result<()> {
    match (img, customization, dst) {
        #[cfg(feature = "sd")]
        (BoardImage::SdFormat { .. }, _, Destination::SdCard(t)) => {
            tokio::task::spawn_blocking(move || bb_flasher::sd::FormatFlasher::new(t).flash())
                .await
                .unwrap()
        }
        #[cfg(feature = "sd")]
        (BoardImage::Image { img, flasher, .. }, customization, Destination::LocalFile(f))
            if flasher == config::Flasher::SdCard =>
        {
            tokio::task::spawn_blocking(move || {
                bb_flasher::sd::Flasher::with_file_dest(
                    img.into_image_fn(),
                    f,
                    customization.sd_customization()?,
                )
                .flash(Some(chan), Some(cancel_sync))
            })
            .await
            .unwrap()
        }
        #[cfg(feature = "sd")]
        (BoardImage::Image { img, flasher, .. }, customization, Destination::SdCard(t))
            if flasher == config::Flasher::SdCard =>
        {
            tokio::task::spawn_blocking(move || {
                bb_flasher::sd::Flasher::new(
                    img.into_image_fn(),
                    t,
                    customization.sd_customization()?,
                )
                .flash(Some(chan), Some(cancel_sync))
            })
            .await
            .unwrap()
        }
        _ => unimplemented!(),
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(crate) enum Destination {
    LocalFile(PathBuf),
    #[cfg(feature = "sd")]
    SdCard(bb_flasher::sd::Target),
}

impl Display for Destination {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Destination::LocalFile(_) => write!(f, "Save To File"),
            #[cfg(feature = "sd")]
            Destination::SdCard(target) => target.fmt(f),
        }
    }
}

impl Destination {
    pub(crate) fn size(&self) -> Option<u64> {
        #[cfg(feature = "sd")]
        if let Destination::SdCard(item) = self {
            return Some(item.size());
        }

        None
    }

    /// Download instead of flashing
    pub(crate) fn is_download_action(&self) -> bool {
        matches!(self, Self::LocalFile(_))
    }

    pub(crate) fn details(&self) -> Vec<(&'static str, String)> {
        match self {
            Self::LocalFile(p) => vec![("Path", p.to_string_lossy().to_string())],
            #[cfg(feature = "sd")]
            Self::SdCard(t) => vec![
                ("Path", t.path().to_string_lossy().to_string()),
                ("Size", pretty_bytes(t.size())),
            ],
        }
    }
}

pub(crate) fn destinations(flasher: config::Flasher, filter: bool) -> Vec<Destination> {
    match flasher {
        #[cfg(feature = "sd")]
        config::Flasher::SdCard => bb_flasher::sd::Target::destinations(filter)
            .into_iter()
            .map(Destination::SdCard)
            .collect(),
        // Only reachable when the crate is built without the `sd` feature.
        #[allow(unreachable_patterns)]
        _ => unimplemented!(),
    }
}

pub(crate) fn file_filter(flasher: config::Flasher) -> &'static [&'static str] {
    match flasher {
        #[cfg(feature = "sd")]
        config::Flasher::SdCard => bb_flasher::sd::Target::FILE_TYPES,
        // Only reachable when the crate is built without the `sd` feature.
        #[allow(unreachable_patterns)]
        _ => unimplemented!(),
    }
}

pub(crate) const fn flasher_supported(flasher: config::Flasher) -> bool {
    match flasher {
        #[cfg(feature = "sd")]
        config::Flasher::SdCard => true,
        // Only reachable when the crate is built without the `sd` feature.
        #[allow(unreachable_patterns)]
        _ => false,
    }
}

#[derive(Clone, Debug)]
pub(crate) enum FlashingCustomization {
    NoneSd,
    LinuxSdSysconfig(crate::persistance::SdSysconfCustomization),
    LinuxSdCloudInit(crate::persistance::SdSysconfCustomization),
    /// T3 GemStone `config.ini`. The flag carries whether the selected image is a desktop variant,
    /// which is what decides whether the VNC fields exist at all.
    T3GemInit {
        config: crate::persistance::T3GemInitCustomization,
        desktop: bool,
    },
}

impl FlashingCustomization {
    pub(crate) fn new(
        flasher: config::Flasher,
        img: &BoardImage,
        app_config: &crate::persistance::GuiConfiguration,
    ) -> Self {
        match flasher {
            config::Flasher::SdCard if img.init_format() == config::InitFormat::Sysconf => {
                Self::LinuxSdSysconfig(
                    app_config
                        .sd_customization
                        .as_ref()
                        .map(|x| x.sysconf_customization().cloned().unwrap_or_default())
                        .unwrap_or_default(),
                )
            }
            config::Flasher::SdCard if img.init_format() == config::InitFormat::CloudInit => {
                Self::LinuxSdCloudInit(
                    app_config
                        .sd_customization
                        .as_ref()
                        .map(|x| x.sysconf_customization().cloned().unwrap_or_default())
                        .unwrap_or_default(),
                )
            }
            flasher if img.init_format().is_gem_init() && flasher == config::Flasher::SdCard => {
                Self::T3GemInit {
                    config: app_config
                        .sd_customization
                        .as_ref()
                        .and_then(|x| x.t3_customization().cloned())
                        .unwrap_or_default(),
                    desktop: img.init_format().supports_vnc(),
                }
            }
            config::Flasher::SdCard => Self::NoneSd,
            #[allow(unreachable_patterns)]
            _ => unimplemented!(),
        }
    }

    pub(crate) fn reset(&mut self) {
        match self {
            Self::LinuxSdSysconfig(_) => *self = Self::LinuxSdSysconfig(Default::default()),
            // Resetting must clear the secrets too, so the whole buffer is replaced rather than
            // having its non-secret fields cleared one by one.
            Self::T3GemInit { desktop, .. } => {
                *self = Self::T3GemInit {
                    config: Default::default(),
                    desktop: *desktop,
                }
            }
            _ => {}
        }
    }

    pub(crate) fn validate(&self) -> bool {
        match self {
            FlashingCustomization::LinuxSdSysconfig(sd_customization) => {
                sd_customization.validate_user()
            }
            // The T3 screen is valid exactly when the file it describes can be produced, so this
            // asks the serializer instead of duplicating its rules.
            FlashingCustomization::T3GemInit { config, desktop } => config.build(*desktop).is_ok(),
            _ => true,
        }
    }

    /// The first problem with the current T3 form, for display next to the disabled NEXT button.
    ///
    /// Returns `None` when the form is valid or is not a T3 form.
    pub(crate) fn validation_error(&self) -> Option<String> {
        match self {
            FlashingCustomization::T3GemInit { config, desktop } => {
                config.build(*desktop).err().map(|e| e.to_string())
            }
            _ => None,
        }
    }

    /// Build the customization the flasher will apply.
    ///
    /// This returns a `Result` rather than falling back to "no customization": a card flashed
    /// without the first-boot file the user configured is a wrong result, not a degraded one, and
    /// it would only be discovered after boot.
    #[cfg(feature = "sd")]
    fn sd_customization(self) -> anyhow::Result<bb_flasher::sd::FlashingSdLinuxConfig> {
        Ok(match self {
            FlashingCustomization::LinuxSdSysconfig(c) => c.sysconfig(),
            FlashingCustomization::LinuxSdCloudInit(c) => c.cloudinit(),
            FlashingCustomization::NoneSd => bb_flasher::sd::FlashingSdLinuxConfig::none(),
            FlashingCustomization::T3GemInit { config, desktop } => {
                let config = config.build(desktop)?;
                bb_flasher::sd::FlashingSdLinuxConfig::t3_gem_init(&config)?
            }
        })
    }
}

#[cfg(target_os = "linux")]
async fn show_notification_xdg_portal(body: &str) -> ashpd::Result<()> {
    let proxy = ashpd::desktop::notification::NotificationProxy::new().await?;

    let app_id = "org.beagleboard.imagingutility";
    proxy
        .add_notification(
            app_id,
            ashpd::desktop::notification::Notification::new("BeagleBoard Imager").body(body),
        )
        .await
}

pub(crate) async fn show_notification(body: String) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    if show_notification_xdg_portal(&body).await.is_ok() {
        return Ok(());
    }

    #[cfg(feature = "notify-rust")]
    if tokio::task::spawn_blocking(move || {
        notify_rust::Notification::new()
            .appname("BeagleBoard Imager")
            .body(&body)
            .finalize()
            .show()
    })
    .await
    .unwrap()
    .is_ok()
    {
        return Ok(());
    };

    Err(anyhow::anyhow!("Failed to send notification"))
}

pub(crate) fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from(
        crate::constants::PACKAGE_QUALIFIER.0,
        crate::constants::PACKAGE_QUALIFIER.1,
        crate::constants::PACKAGE_QUALIFIER.2,
    )
}

pub(crate) fn log_file_path() -> PathBuf {
    let dirs = project_dirs().unwrap();
    dirs.cache_dir().with_file_name(format!(
        "{}.{}.{}.log",
        PACKAGE_QUALIFIER.0, PACKAGE_QUALIFIER.1, PACKAGE_QUALIFIER.2
    ))
}

pub(crate) fn pretty_bytes(bytes: u64) -> String {
    const UNITS: [&str; 7] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];

    if bytes == 0 {
        return "0 B".to_string();
    }

    let mut size = bytes as f64;
    let mut unit = 0;

    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{:.2} {}", size, UNITS[unit])
    }
}

/// Return customization enum variant for cases where no customization is present
pub(crate) fn no_customization(
    flasher: config::Flasher,
    img: &BoardImage,
) -> Option<FlashingCustomization> {
    match flasher {
        config::Flasher::SdCard
            if img.init_format() == config::InitFormat::Sysconf
                || img.init_format() == config::InitFormat::CloudInit
                || img.init_format().is_gem_init() =>
        {
            None
        }
        config::Flasher::SdCard => Some(FlashingCustomization::NoneSd),
    }
}

pub(crate) fn pretty_duration(d: Duration) -> String {
    let secs = d.as_secs();

    if secs >= 60 {
        format!("{}:{:02}", secs / 60, secs % 60)
    } else {
        format!("{}s", secs)
    }
}

pub(crate) fn app_title(_: &crate::BBImager) -> String {
    if cfg!(feature = "pre-release") {
        format!("{} (pre-release)", constants::APP_NAME)
    } else {
        format!("{} v{}", constants::APP_NAME, env!("CARGO_PKG_VERSION"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OsImageId {
    Format,
    // points to parent
    Local(config::Flasher),
    // points to OsImage
    OsImage(i64),
    OsSublist((i64, config::Flasher)),
}

#[derive(Debug, Clone)]
pub(crate) struct OsImageItem {
    pub(crate) id: OsImageId,
    pub(crate) icon: Option<url::Url>,
    pub(crate) label: Cow<'static, str>,
}

impl From<crate::db::OsImageListItem> for OsImageItem {
    fn from(value: crate::db::OsImageListItem) -> Self {
        Self {
            id: OsImageId::OsImage(value.id),
            icon: Some(value.icon),
            label: Cow::Owned(value.name),
        }
    }
}

impl From<crate::db::OsSublistListItem> for OsImageItem {
    fn from(value: crate::db::OsSublistListItem) -> Self {
        Self {
            id: OsImageId::OsSublist((value.id, value.flasher)),
            icon: Some(value.icon),
            label: Cow::Owned(value.name),
        }
    }
}

impl OsImageItem {
    pub(crate) fn format(label: Cow<'static, str>) -> Self {
        Self {
            id: OsImageId::Format,
            icon: None,
            label,
        }
    }

    pub(crate) fn local(flasher: config::Flasher) -> Self {
        Self {
            id: OsImageId::Local(flasher),
            icon: None,
            label: Cow::Borrowed("Select Local Image"),
        }
    }

    pub(crate) const fn is_sublist(&self) -> bool {
        matches!(self.id, OsImageId::OsSublist(_))
    }

    pub(crate) fn label(&self) -> &str {
        &self.label
    }
}

#[derive(Debug)]
pub(crate) enum DestinationItem<'a> {
    SaveToFile(String),
    Destination(&'a Destination),
}

impl<'a> std::fmt::Display for DestinationItem<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DestinationItem::SaveToFile(_) => write!(f, "Save To File"),
            DestinationItem::Destination(d) => d.fmt(f),
        }
    }
}

fn normalize_file_dest(name: &str) -> String {
    if let Some(stripped) = name.strip_suffix(".zip") {
        return stripped.to_string();
    }

    if let Some(pos) = name.rfind(".img.") {
        return name[..pos + 4].to_string();
    }

    name.to_string()
}

impl<'a> DestinationItem<'a> {
    pub(crate) fn msg(&'a self) -> BBImagerMessage {
        match self {
            DestinationItem::SaveToFile(x) => {
                BBImagerMessage::SelectFileDest(normalize_file_dest(x))
            }
            DestinationItem::Destination(d) => BBImagerMessage::SelectDest((*d).clone()),
        }
    }

    pub(crate) fn is_selected(&'a self, dst: &'a Destination) -> bool {
        match self {
            DestinationItem::SaveToFile(_) => false,
            DestinationItem::Destination(d) => dst.eq(d),
        }
    }

    pub(crate) fn subtitle(&self) -> Option<String> {
        match self {
            DestinationItem::SaveToFile(_) => None,
            DestinationItem::Destination(d) => d.size().map(crate::helpers::pretty_bytes),
        }
    }
}

pub(crate) fn fetch_images(
    downloader: &bb_downloader::Downloader,
    iter: impl IntoIterator<Item = url::Url>,
) -> iced::Task<BBImagerMessage> {
    let tasks = iter.into_iter().map(|icon| {
        let downloader = downloader.clone();
        let icon_clone = icon.clone();
        let icon_clone2 = icon.clone();
        iced::Task::perform(
            async move { downloader.download(icon_clone).await },
            move |p| match p {
                Ok(p) => BBImagerMessage::ResolveImage(icon_clone2, p),
                Err(_) => {
                    tracing::warn!("Failed to fetch image {}", icon);
                    BBImagerMessage::Null
                }
            },
        )
    });

    iced::Task::batch(tasks)
}

/// Whether a remote config URL is the T3 image catalog.
///
/// Matched on host rather than on the exact URL so a mirror or a path change still takes the
/// strict adapter instead of silently falling back to the BeagleBoard parser.
fn is_t3_catalog(url: &Url) -> bool {
    bb_config::t3::T3_CATALOG_URL
        .parse::<Url>()
        .ok()
        .and_then(|canonical| Some((canonical.host_str()?.to_owned(), url.host_str()?)))
        .is_some_and(|(canonical_host, host)| canonical_host == host)
}

/// Fetch a remote config, routing the T3 catalog through its strict adapter.
///
/// The T3 document must never be parsed as a [`bb_config::config::Config`]. It has no `flasher`
/// field on devices and declares `init_format: "systemd"` on images, and both structs are parsed
/// with `VecSkipError` — so the legacy path does not fail, it yields an **empty board and image
/// list**. That is why the board never appeared. Anything that is not the T3 catalog keeps the
/// original path.
pub(crate) async fn fetch_remote_config(
    downloader: &bb_downloader::Downloader,
    url: Url,
) -> std::io::Result<bb_config::config::Config> {
    if !is_t3_catalog(&url) {
        return Ok(downloader.download_json_no_cache(url).await?);
    }

    let raw: bb_config::t3::RawT3Catalog = downloader.download_json_no_cache(url.clone()).await?;

    // Product scope: T3-GEM-O1 and BeagleY-AI, and nothing else. The catalog also publishes a
    // tagless "No filtering" pseudo-device, which carries neither board tag and is therefore out
    // of scope by construction rather than by a name check.
    let parsed = bb_config::t3::validate_catalog(
        raw,
        bb_config::t3::ProductScope::T3AndBeagleY,
        url.as_str(),
    )
    .map_err(|e| std::io::Error::other(format!("T3 catalog rejected: {e}")))?;

    // Every dropped or downgraded entry carries a JSON path. Surfacing them is what keeps a
    // shrinking catalog visible instead of looking like a normal, smaller list.
    for diagnostic in &parsed.diagnostics {
        tracing::warn!("T3 catalog: {diagnostic}");
    }
    if parsed.rejected_boards > 0 || parsed.rejected_images > 0 {
        tracing::warn!(
            "T3 catalog: dropped {} board(s) and {} image(s)",
            parsed.rejected_boards,
            parsed.rejected_images
        );
    }

    let config = bb_config::t3::catalog_to_config(&parsed.catalog);
    tracing::info!(
        "T3 catalog: {} board(s) and {} image(s) in scope",
        config.imager.devices.len(),
        config.os_list.len()
    );

    Ok(config)
}

pub(crate) fn fetch_remote_subitems(
    items: impl IntoIterator<Item = (i64, Url)>,
    downloader: bb_downloader::Downloader,
) -> iced::Task<BBImagerMessage> {
    let temp = items.into_iter().map(move |(id, url)| {
        let url_clone = url.clone();
        let dl = downloader.clone();
        iced::Task::perform(
            async move { dl.download_json_no_cache(url_clone).await },
            move |x| match x {
                Ok(json) => BBImagerMessage::ResolveRemoteSubitemItem {
                    item: json,
                    target: id,
                },
                Err(e) => {
                    tracing::error!("Failed to get remote item {}: {e}", url.as_str());
                    BBImagerMessage::Null
                }
            },
        )
    });

    iced::Task::batch(temp)
}

pub(crate) fn sd_modifications_common(
    x: &crate::persistance::SdSysconfCustomization,
) -> Vec<&'static str> {
    let mut ans = Vec::new();

    if x.user.is_some() {
        ans.push("• User account configured");
    }
    if x.wifi.is_some() {
        ans.push("• Wifi configured");
    }
    if x.hostname.is_some() {
        ans.push("• Hostname configured");
    }
    if x.keymap.is_some() {
        ans.push("• Keymap configured");
    }
    if x.timezone.is_some() {
        ans.push("• Timezone configured");
    }
    if x.ssh.is_some() {
        ans.push("• SSH Key configured");
    }

    ans
}

pub(crate) async fn blocking_future<F, T>(f: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f).await.unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistance::{
        GuiConfiguration, SdCustomizationUser, SdCustomizationWifi, SdSysconfCustomization,
    };

    #[test]
    fn pretty_bytes_scales_units() {
        assert_eq!(pretty_bytes(0), "0 B");
        assert_eq!(pretty_bytes(512), "512 B");
        assert_eq!(pretty_bytes(1024), "1.00 KiB");
        assert_eq!(pretty_bytes(1536), "1.50 KiB");
        assert_eq!(pretty_bytes(1024 * 1024), "1.00 MiB");
        assert_eq!(pretty_bytes(1024 * 1024 * 1024), "1.00 GiB");
    }

    #[test]
    fn pretty_duration_formats_minutes_and_seconds() {
        assert_eq!(pretty_duration(Duration::from_secs(0)), "0s");
        assert_eq!(pretty_duration(Duration::from_secs(45)), "45s");
        assert_eq!(pretty_duration(Duration::from_secs(60)), "1:00");
        assert_eq!(pretty_duration(Duration::from_secs(125)), "2:05");
    }

    #[test]
    fn normalize_file_dest_strips_known_suffixes() {
        assert_eq!(normalize_file_dest("os.zip"), "os");
        assert_eq!(normalize_file_dest("os.img.xz"), "os.img");
        assert_eq!(normalize_file_dest("os.img.gz"), "os.img");
        assert_eq!(normalize_file_dest("plain.txt"), "plain.txt");
    }

    #[test]
    fn flasher_supported_matches_enabled_features() {
        // const fn whose arms are feature-gated; compare against cfg! so the
        // assertion holds under any feature set the suite is compiled with.
        assert_eq!(
            flasher_supported(config::Flasher::SdCard),
            cfg!(feature = "sd")
        );
    }

    #[test]
    fn sd_modifications_common_lists_configured_fields() {
        assert!(sd_modifications_common(&SdSysconfCustomization::default()).is_empty());

        let full = SdSysconfCustomization::default()
            .update_hostname(Some("h".into()))
            .update_timezone(Some("UTC".parse().unwrap()))
            .update_keymap(Some("us".into()))
            .update_ssh(Some("k".into()))
            .update_user(Some(SdCustomizationUser::new("u".into(), "p".into())))
            .update_wifi(Some(SdCustomizationWifi::default()));
        let mods = sd_modifications_common(&full);
        assert_eq!(mods.len(), 6);
        assert!(mods.contains(&"• User account configured"));
        assert!(mods.contains(&"• Wifi configured"));
        assert!(mods.contains(&"• SSH Key configured"));
    }

    #[test]
    fn no_customization_covers_non_configurable_flashers() {
        let img = BoardImage::format();
        assert!(matches!(
            no_customization(config::Flasher::SdCard, &img),
            Some(FlashingCustomization::NoneSd)
        ));
    }

    #[test]
    fn flashing_customization_new_selects_variant_by_flasher() {
        // A format image has init_format None, so SD falls through to NoneSd.
        let img = BoardImage::format();
        let cfg = GuiConfiguration::default();

        assert!(matches!(
            FlashingCustomization::new(config::Flasher::SdCard, &img, &cfg),
            FlashingCustomization::NoneSd
        ));
    }

    #[test]
    fn flashing_customization_validate_checks_user() {
        assert!(FlashingCustomization::NoneSd.validate());
        assert!(
            FlashingCustomization::LinuxSdSysconfig(SdSysconfCustomization::default()).validate()
        );
        let root = SdSysconfCustomization::default()
            .update_user(Some(SdCustomizationUser::new("root".into(), "p".into())));
        assert!(!FlashingCustomization::LinuxSdSysconfig(root).validate());
    }

    #[test]
    fn flashing_customization_reset_restores_defaults() {
        let mut sysconf = FlashingCustomization::LinuxSdSysconfig(
            SdSysconfCustomization::default().update_hostname(Some("h".into())),
        );
        sysconf.reset();
        match sysconf {
            FlashingCustomization::LinuxSdSysconfig(c) => assert!(c.hostname.is_none()),
            _ => panic!("variant should be preserved"),
        }

        // Variants without inner state are left untouched.
        let mut none = FlashingCustomization::NoneSd;
        none.reset();
        assert!(matches!(none, FlashingCustomization::NoneSd));
    }

    #[test]
    fn board_image_format_accessors() {
        let img = BoardImage::format();
        assert_eq!(
            img.description(),
            Some("Format a SD Card to FAT32 for reuse.")
        );
        assert_eq!(img.flasher(), config::Flasher::SdCard);
        assert_eq!(img.init_format(), config::InitFormat::None);
        assert_eq!(img.info_text(), None);
        assert_eq!(img.file_name(), None);
        assert_eq!(img.details(), &[("Format", "FAT32".to_string())]);
        assert!(img.supported_init_formats().is_empty());
        assert!(img.support().is_none());
        assert!(matches!(img.icon(), BoardImageIcon::Format));
        assert_eq!(img.to_string(), "Format SD Card");
    }

    #[test]
    fn board_image_local_reads_file_metadata() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"0123456789").unwrap();

        let img = BoardImage::local(file.path().to_path_buf(), config::Flasher::SdCard);
        assert_eq!(img.flasher(), config::Flasher::SdCard);
        assert_eq!(img.init_format(), config::InitFormat::None);
        assert!(matches!(img.icon(), BoardImageIcon::Local));
        assert!(img.description().is_none());
        assert!(img.file_name().is_some_and(|n| !n.is_empty()));

        let details = img.details();
        assert!(details.iter().any(|(k, _)| *k == "Path"));
        assert!(details.iter().any(|(k, v)| *k == "Size" && v == "10"));
        // A local file carries no catalog metadata, so the format cannot be derived from it. For an
        // SD target the user picks one instead (see `board_image_update_init_format_on_image`),
        // which is why both BeagleBoard formats are offered rather than none.
        //
        // The T3 GemInit formats are deliberately absent: writing `config.ini` onto an arbitrary
        // local image would be guessing that the image is a T3 one, and a first-boot file the image
        // does not consume is the failure this whole phase exists to avoid.
        assert_eq!(
            img.supported_init_formats(),
            &[config::InitFormat::Sysconf, config::InitFormat::CloudInit]
        );
    }

    #[test]
    fn board_image_update_init_format_on_image() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"x").unwrap();
        let mut img = BoardImage::local(file.path().to_path_buf(), config::Flasher::SdCard);
        img.update_init_format(config::InitFormat::Sysconf);
        assert_eq!(img.init_format(), config::InitFormat::Sysconf);
    }

    #[test]
    fn destination_local_file_behaviour() {
        let dst = Destination::LocalFile(PathBuf::from("/tmp/os.img"));
        assert!(dst.is_download_action());
        assert_eq!(dst.size(), None);
        assert_eq!(dst.details(), vec![("Path", "/tmp/os.img".to_string())]);
        assert_eq!(dst.to_string(), "Save To File");
    }

    #[test]
    fn destination_item_save_to_file() {
        let item = DestinationItem::SaveToFile("os.img.xz".to_string());
        let other = Destination::LocalFile(PathBuf::from("/tmp/x"));

        assert_eq!(item.to_string(), "Save To File");
        assert!(!item.is_selected(&other));
        assert!(item.subtitle().is_none());
        match item.msg() {
            BBImagerMessage::SelectFileDest(name) => assert_eq!(name, "os.img"),
            other => panic!("expected SelectFileDest, got {other:?}"),
        }
    }

    #[test]
    fn destination_item_wraps_destination() {
        let dst = Destination::LocalFile(PathBuf::from("/tmp/os.img"));
        let other = Destination::LocalFile(PathBuf::from("/tmp/other.img"));
        let item = DestinationItem::Destination(&dst);

        assert_eq!(item.to_string(), "Save To File");
        assert!(item.is_selected(&dst));
        assert!(!item.is_selected(&other));
        // LocalFile has no size, so no subtitle.
        assert!(item.subtitle().is_none());
    }

    #[test]
    fn os_image_item_constructors_and_predicates() {
        let local = OsImageItem::local(config::Flasher::SdCard);
        assert_eq!(local.id, OsImageId::Local(config::Flasher::SdCard));
        assert!(!local.is_sublist());
        assert_eq!(local.label(), "Select Local Image");

        let format = OsImageItem::format("Format".into());
        assert_eq!(format.id, OsImageId::Format);
        assert!(!format.is_sublist());
        assert_eq!(format.label(), "Format");
    }

    #[test]
    fn os_image_item_from_db_items() {
        let icon = Url::parse("https://example.com/icon.png").unwrap();

        let image: OsImageItem = crate::db::OsImageListItem {
            id: 5,
            icon: icon.clone(),
            name: "Debian".to_string(),
        }
        .into();
        assert_eq!(image.id, OsImageId::OsImage(5));
        assert!(!image.is_sublist());
        assert_eq!(image.label(), "Debian");

        let sublist: OsImageItem = crate::db::OsSublistListItem {
            id: 7,
            icon,
            name: "More".to_string(),
            flasher: config::Flasher::SdCard,
        }
        .into();
        assert_eq!(
            sublist.id,
            OsImageId::OsSublist((7, config::Flasher::SdCard))
        );
        assert!(sublist.is_sublist());
    }

    #[test]
    fn system_keymap_is_never_empty() {
        // Falls back to "us" when the locale cannot be resolved.
        assert!(!system_keymap().is_empty());
    }
}
