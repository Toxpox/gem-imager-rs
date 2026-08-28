use std::io::{self, Read as _, Seek as _, Write as _};
use std::{borrow::Cow, fmt::Display, path::PathBuf, sync::LazyLock, time::Duration};

use crate::{GemImagerMessage, PACKAGE_QUALIFIER, constants};
use gem_config::config;
use gem_flasher::DownloadFlashingStatus;
#[cfg(any(feature = "sd", feature = "dfu"))]
use gem_flasher::GemFlasherTarget;
use gem_flasher::img::OsImage;
use std::sync::mpsc;
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
            img: gem_flasher::LocalImage::new(path.into()).into(),
            flasher,
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
        downloader: gem_downloader::Downloader,
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

    pub(crate) const fn supports_dfu(&self) -> bool {
        matches!(self, Self::Image { .. })
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

pub(crate) fn system_language() -> Option<gem_i18n::Lang> {
    static SYSTEM_LANGUAGE: LazyLock<Option<gem_i18n::Lang>> = LazyLock::new(|| {
        let prefs = whoami::lang_prefs().ok()?;

        prefs
            .message_langs()
            .find_map(|lang| gem_i18n::Lang::from_code(&lang.to_string()))
    });
    *SYSTEM_LANGUAGE
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

#[derive(Debug, Clone, Default)]
pub(crate) struct HostWifiPrefill {
    pub(crate) ssid: Option<String>,
    pub(crate) password: Option<gem_flasher::t3_gem_init::Secret>,
    pub(crate) country: Option<String>,
}

pub(crate) fn detect_host_wifi() -> HostWifiPrefill {
    use gem_host_wifi::PasswordOutcome;

    let mut out = HostWifiPrefill::default();

    let wifi = match gem_host_wifi::detect_current_wifi() {
        Ok(wifi) => wifi,
        Err(e) => {
            tracing::info!("Host Wi-Fi autofill unavailable: {e}");
            return out;
        }
    };

    out.ssid = wifi.ssid.as_utf8().map(str::to_owned);
    out.country = wifi.country.as_ref().map(|c| c.code.as_str().to_owned());

    if wifi.security.can_carry_passphrase() {
        match gem_host_wifi::read_saved_password(&wifi.network) {
            Ok(PasswordOutcome::Found(secret)) => out.password = Some(secret),
            Ok(other) => tracing::info!("No saved Wi-Fi password to autofill: {other:?}"),
            Err(e) => tracing::info!("Reading the saved Wi-Fi password failed: {e}"),
        }
    }

    out
}

#[derive(Debug, Clone, serde::Serialize)]
#[cfg_attr(not(feature = "sd"), allow(dead_code))]
pub(crate) struct RemoteImage {
    name: Box<str>,
    url: Box<url::Url>,
    #[serde(with = "const_hex")]
    archive_sha256: [u8; 32],
    archive_size: Option<u64>,
    #[serde(skip)]
    extract_sha256: Option<[u8; 32]>,
    extract_size: u64,
    #[serde(skip)]
    downloader: gem_downloader::Downloader,
}

impl RemoteImage {
    pub(crate) fn new(
        name: Box<str>,
        url: Box<url::Url>,
        archive_sha256: [u8; 32],
        archive_size: Option<u64>,
        extract_sha256: Option<[u8; 32]>,
        extract_size: u64,
        downloader: gem_downloader::Downloader,
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

    #[cfg_attr(not(feature = "sd"), allow(dead_code))]
    fn extract_gate(&self) -> gem_flasher::img::ExtractGate {
        match self.extract_sha256 {
            Some(sha256) => gem_flasher::img::ExtractGate::Declared(
                gem_flasher::img::ExtractedIntegrity::new(self.extract_size, sha256),
            ),
            None => gem_flasher::img::ExtractGate::UndeclaredLegacyCatalog,
        }
    }

    fn file_name(&self) -> &str {
        self.url.path_segments().unwrap().next_back().unwrap()
    }

    #[cfg_attr(not(feature = "dfu"), allow(dead_code))]
    fn archive_cache_growth_estimate(&self) -> u64 {
        if self
            .downloader
            .check_cache_from_sha(self.archive_sha256)
            .is_some()
        {
            0
        } else {
            self.archive_size
                .unwrap_or(self.downloader.policy().max_stream_body)
        }
    }

    #[cfg_attr(not(feature = "sd"), allow(dead_code))]
    fn into_image_fn(
        self,
        cancel: gem_helper::cancel::CancellationToken,
    ) -> impl FnOnce() -> io::Result<(Box<dyn io::Read + Send>, u64)> + Send {
        let rt = tokio::runtime::Handle::current();
        move || {
            if cancel.is_cancelled() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "flash cancelled",
                ));
            }

            let downloader = self.downloader.clone();
            let cached_path = downloader.check_cache_from_sha(self.archive_sha256);
            let archive_growth = if cached_path.is_some() {
                0
            } else {
                self.archive_size
                    .unwrap_or(downloader.policy().max_stream_body)
            };
            let staging = crate::staging::StagingImage::create(
                self.extract_size.saturating_add(archive_growth),
            )
            .map_err(io::Error::other)?;

            let path = if let Some(path) = cached_path {
                tracing::info!("Found the remote image in cache");
                path
            } else {
                tracing::info!("Remote image not found in cache. Downloading before flashing");
                let (writer, _reader) = gem_helper::file_stream::file_stream()?;
                let integrity = gem_downloader::ArchiveIntegrity {
                    sha256: self.archive_sha256,
                    size: self.archive_size,
                };

                rt.block_on(async {
                    let download =
                        downloader.download_to_stream(*self.url.clone(), integrity, writer);
                    tokio::pin!(download);

                    loop {
                        tokio::select! {
                            result = &mut download => {
                                break result.map_err(|e| {
                                    let msg = format!("Error while downloading Os Image: {e}");
                                    tracing::error!("{}", &msg);
                                    io::Error::other(msg)
                                });
                            }
                            () = tokio::time::sleep(Duration::from_millis(100)) => {
                                if cancel.is_cancelled() {
                                    break Err(io::Error::new(
                                        io::ErrorKind::Interrupted,
                                        "flash cancelled",
                                    ));
                                }
                            }
                        }
                    }
                })?;

                downloader
                    .check_cache_from_sha(self.archive_sha256)
                    .ok_or_else(|| {
                        io::Error::other("verified image was not published to the cache")
                    })?
            };

            let mut image = OsImage::from_path(&path, self.extract_gate())?;
            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(staging.path())?;
            let mut written = 0u64;
            let mut buf = vec![0u8; 1024 * 1024];
            loop {
                if cancel.is_cancelled() {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "flash cancelled",
                    ));
                }
                let count = image.read(&mut buf)?;
                if count == 0 {
                    break;
                }
                let next_written = written.saturating_add(count as u64);
                if next_written > self.extract_size {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "extracted image size mismatch: expected {}, got at least {next_written}",
                            self.extract_size
                        ),
                    ));
                }
                file.write_all(&buf[..count])?;
                written = next_written;
            }
            if written != self.extract_size {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "extracted image size mismatch: expected {}, got {written}",
                        self.extract_size
                    ),
                ));
            }
            file.flush()?;
            file.sync_all()?;
            file.rewind()?;

            Ok((
                Box::new(StagedRemoteImage {
                    _staging: staging,
                    file,
                }) as Box<dyn io::Read + Send>,
                self.extract_size,
            ))
        }
    }
}

#[cfg_attr(not(feature = "sd"), allow(dead_code))]
struct StagedRemoteImage {
    file: std::fs::File,
    _staging: crate::staging::StagingImage,
}

#[cfg_attr(not(feature = "sd"), allow(dead_code))]
type ImageReader = Box<dyn io::Read + Send>;
#[cfg_attr(not(feature = "sd"), allow(dead_code))]
type ImageResolver = Box<dyn FnOnce() -> io::Result<(ImageReader, u64)> + Send>;

impl io::Read for StagedRemoteImage {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

impl std::fmt::Display for RemoteImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) enum SelectedImage {
    LocalImage(gem_flasher::LocalImage),
    RemoteImage(Box<RemoteImage>),
}

impl SelectedImage {
    fn file_name(&self) -> String {
        match self {
            Self::LocalImage(x) => x.file_name().to_string_lossy().to_string(),
            Self::RemoteImage(x) => x.file_name().to_string(),
        }
    }

    #[cfg_attr(not(feature = "dfu"), allow(dead_code))]
    fn staging_size_estimate(&self) -> u64 {
        match self {
            Self::RemoteImage(x) => x
                .extract_size
                .saturating_mul(2)
                .saturating_add(x.archive_cache_growth_estimate()),
            Self::LocalImage(x) => std::fs::metadata(x.path()).map(|m| m.len()).unwrap_or(0),
        }
    }

    #[cfg_attr(not(feature = "sd"), allow(dead_code))]
    fn into_image_fn(self, cancel: gem_helper::cancel::CancellationToken) -> ImageResolver {
        match self {
            SelectedImage::LocalImage(x) => Box::new(move || {
                if cancel.is_cancelled() {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "flash cancelled",
                    ));
                }
                let (image, size) = x.into_image_fn()()?;
                Ok((Box::new(image) as ImageReader, size))
            }),
            SelectedImage::RemoteImage(x) => Box::new((*x).into_image_fn(cancel)),
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

impl From<gem_flasher::LocalImage> for SelectedImage {
    fn from(value: gem_flasher::LocalImage) -> Self {
        Self::LocalImage(value)
    }
}

pub(crate) async fn flash(
    img: BoardImage,
    customization: FlashingCustomization,
    dst: Destination,
    #[cfg_attr(not(feature = "sd"), allow(unused_variables))] chan: mpsc::SyncSender<
        DownloadFlashingStatus,
    >,
    #[cfg_attr(not(feature = "sd"), allow(unused_variables))]
    cancel_sync: gem_helper::cancel::CancellationToken,
) -> anyhow::Result<()> {
    if let Some((title, _)) = dst.unavailable_reason() {
        return Err(match title {
            gem_i18n::Msg::DfuPermissionTitle => anyhow::anyhow!(
                "DFU device permission denied: the board is present but cannot be opened"
            ),
            _ => anyhow::anyhow!(
                "DFU device driver missing: no WinUSB-compatible driver is bound to the board"
            ),
        });
    }

    let _awake = crate::keep_awake::KeepAwake::acquire();

    match (img, customization, dst) {
        #[cfg(all(feature = "dfu", feature = "sd"))]
        (BoardImage::Image { img, .. }, customization, Destination::T3Dfu(target)) => {
            let identifier = target.identifier().into_owned();
            let customization = customization.sd_customization()?;

            tokio::task::spawn_blocking(move || {
                let estimate = img.staging_size_estimate();
                let staging = crate::staging::StagingImage::create(estimate)?;
                tracing::info!("Staging the customized image in the private application cache");

                gem_flasher::sd::Flasher::with_file_dest(
                    img.into_image_fn(cancel_sync.clone()),
                    staging.path().to_path_buf(),
                    customization,
                )
                .flash(Some(chan.clone()), Some(cancel_sync.clone()))?;

                gem_flasher::dfu::Flasher::from_staging_file(
                    staging.path(),
                    &identifier,
                    Some(cancel_sync),
                )?
                .flash(Some(chan))
            })
            .await
            .unwrap_or_else(|join_error| {
                Err(anyhow::anyhow!(
                    "the DFU write ended unexpectedly: {join_error}"
                ))
            })
        }
        #[cfg(feature = "sd")]
        (BoardImage::SdFormat { .. }, _, Destination::SdCard(t)) => {
            tokio::task::spawn_blocking(move || gem_flasher::sd::FormatFlasher::new(t).flash())
                .await
                .unwrap()
        }
        #[cfg(feature = "sd")]
        (BoardImage::Image { img, flasher, .. }, customization, Destination::LocalFile(f))
            if flasher == config::Flasher::SdCard =>
        {
            tokio::task::spawn_blocking(move || {
                gem_flasher::sd::Flasher::with_file_dest(
                    img.into_image_fn(cancel_sync.clone()),
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
                gem_flasher::sd::Flasher::new(
                    img.into_image_fn(cancel_sync.clone()),
                    t,
                    customization.sd_customization()?,
                )
                .flash(Some(chan), Some(cancel_sync))
            })
            .await
            .unwrap()
        }
        _ => anyhow::bail!("no write path is compiled for this image and destination combination"),
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(crate) enum Destination {
    LocalFile(PathBuf),
    #[cfg(feature = "sd")]
    SdCard(gem_flasher::sd::Target),
    #[cfg(feature = "dfu")]
    T3Dfu(gem_flasher::dfu::Target),
}

impl Display for Destination {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Destination::LocalFile(_) => write!(f, "Save To File"),
            #[cfg(feature = "sd")]
            Destination::SdCard(target) => target.fmt(f),
            #[cfg(feature = "dfu")]
            Destination::T3Dfu(target) => target.fmt(f),
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

    pub(crate) fn is_download_action(&self) -> bool {
        matches!(self, Self::LocalFile(_))
    }

    #[cfg(feature = "dfu")]
    pub(crate) fn unavailable_reason(&self) -> Option<(gem_i18n::Msg, gem_i18n::Msg)> {
        let Self::T3Dfu(target) = self else {
            return None;
        };

        match target.access() {
            gem_flasher::dfu::DeviceAccess::Available => None,
            gem_flasher::dfu::DeviceAccess::PermissionDenied => Some((
                gem_i18n::Msg::DfuPermissionTitle,
                gem_i18n::Msg::DfuPermissionBody,
            )),
            gem_flasher::dfu::DeviceAccess::DriverMissing => Some((
                gem_i18n::Msg::WinusbDriverMissingTitle,
                gem_i18n::Msg::WinusbDriverMissingBody,
            )),
        }
    }

    #[cfg(not(feature = "dfu"))]
    pub(crate) fn unavailable_reason(&self) -> Option<(gem_i18n::Msg, gem_i18n::Msg)> {
        None
    }

    pub(crate) fn is_dfu(&self) -> bool {
        #[cfg(feature = "dfu")]
        return matches!(self, Self::T3Dfu(_));
        #[cfg(not(feature = "dfu"))]
        return false;
    }

    pub(crate) fn details(&self) -> Vec<(&'static str, String)> {
        match self {
            Self::LocalFile(p) => vec![("Path", p.to_string_lossy().to_string())],
            #[cfg(feature = "sd")]
            Self::SdCard(t) => vec![
                ("Path", t.path().to_string_lossy().to_string()),
                ("Size", pretty_bytes(t.size())),
            ],
            #[cfg(feature = "dfu")]
            Self::T3Dfu(t) => vec![
                ("USB Port", t.identifier().into_owned()),
                ("Target", "Onboard eMMC (DFU)".to_owned()),
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct WriteMethods {
    pub(crate) sd: bool,
    pub(crate) dfu: bool,
}

impl WriteMethods {
    pub(crate) fn resolve(board: &crate::db::Board, img: &BoardImage) -> Self {
        Self {
            sd: cfg!(feature = "sd") && board.flasher == config::Flasher::SdCard,
            dfu: cfg!(feature = "dfu") && board.emmc_dfu && img.supports_dfu(),
        }
    }

    pub(crate) const fn is_empty(self) -> bool {
        !self.sd && !self.dfu
    }
}

pub(crate) fn destinations(methods: WriteMethods, filter: bool) -> Vec<Destination> {
    #[cfg_attr(not(feature = "sd"), allow(unused_mut))]
    let mut out: Vec<Destination> = Vec::new();

    #[cfg(feature = "sd")]
    if methods.sd {
        out.extend(
            gem_flasher::sd::Target::destinations(filter)
                .into_iter()
                .map(Destination::SdCard),
        );
    }

    #[cfg(feature = "dfu")]
    if methods.dfu {
        out.extend(
            gem_flasher::dfu::Target::destinations(filter)
                .into_iter()
                .map(Destination::T3Dfu),
        );
    }

    let _ = (methods, filter);
    out
}

pub(crate) fn keep_selected_destination(
    selected: Option<Destination>,
    available: &[Destination],
) -> Option<Destination> {
    match selected {
        Some(Destination::LocalFile(p)) => Some(Destination::LocalFile(p)),
        #[cfg_attr(not(any(feature = "sd", feature = "dfu")), allow(unreachable_patterns))]
        Some(dest) if available.contains(&dest) => Some(dest),
        #[cfg_attr(not(any(feature = "sd", feature = "dfu")), allow(unreachable_patterns))]
        Some(dest) => {
            tracing::info!("Clearing the selected destination: {dest} is no longer present");
            None
        }
        None => None,
    }
}

pub(crate) fn file_filter(flasher: config::Flasher) -> &'static [&'static str] {
    match flasher {
        #[cfg(feature = "sd")]
        config::Flasher::SdCard => gem_flasher::sd::Target::FILE_TYPES,
        #[allow(unreachable_patterns)]
        _ => unreachable!(
            "file filter requested for {flasher:?}, which has no write path in this build"
        ),
    }
}

pub(crate) const fn flasher_supported(flasher: config::Flasher) -> bool {
    match flasher {
        #[cfg(feature = "sd")]
        config::Flasher::SdCard => true,
        #[allow(unreachable_patterns)]
        _ => false,
    }
}

#[derive(Clone, Debug)]
pub(crate) enum FlashingCustomization {
    NoneSd,
    LinuxSdSysconfig(crate::persistance::SdSysconfCustomization),
    LinuxSdCloudInit(crate::persistance::SdSysconfCustomization),
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
            _ => unreachable!(
                "customization requested for {flasher:?}, which has no write path in this build"
            ),
        }
    }

    pub(crate) fn reset(&mut self) {
        match self {
            Self::LinuxSdSysconfig(_) => *self = Self::LinuxSdSysconfig(Default::default()),
            Self::T3GemInit { desktop, .. } => {
                *self = Self::T3GemInit {
                    config: Default::default(),
                    desktop: *desktop,
                }
            }
            _ => {}
        }
    }

    pub(crate) fn enable_wifi(&mut self) {
        match self {
            Self::LinuxSdSysconfig(c) | Self::LinuxSdCloudInit(c) if c.wifi.is_none() => {
                c.wifi = Some(crate::persistance::SdCustomizationWifi::default());
            }
            Self::T3GemInit { config, .. } if config.wifi.is_none() => {
                config.wifi = Some(crate::persistance::T3WifiCustomization::default());
            }
            _ => {}
        }
    }

    pub(crate) fn wifi_enabled(&self) -> bool {
        match self {
            Self::LinuxSdSysconfig(c) | Self::LinuxSdCloudInit(c) => c.wifi.is_some(),
            Self::T3GemInit { config, .. } => config.wifi.is_some(),
            _ => false,
        }
    }

    pub(crate) fn disable_wifi(&mut self) {
        match self {
            Self::LinuxSdSysconfig(c) | Self::LinuxSdCloudInit(c) => c.wifi = None,
            Self::T3GemInit { config, .. } => config.wifi = None,
            _ => {}
        }
    }

    pub(crate) fn apply_wifi_prefill(&mut self, prefill: HostWifiPrefill) {
        match self {
            Self::LinuxSdSysconfig(c) | Self::LinuxSdCloudInit(c) => {
                if let Some(wifi) = c.wifi.as_mut() {
                    if let Some(ssid) = prefill.ssid {
                        wifi.ssid = ssid;
                    }
                    if let Some(password) = prefill.password {
                        wifi.password = password;
                    }
                }
            }
            Self::T3GemInit { config, .. } => {
                if let Some(wifi) = config.wifi.as_mut() {
                    if let Some(ssid) = prefill.ssid {
                        wifi.ssid = ssid;
                    }
                    if let Some(password) = prefill.password {
                        wifi.password = password;
                    }
                    if let Some(country) = prefill.country {
                        wifi.country = country;
                    }
                }
            }
            _ => {}
        }
    }

    pub(crate) fn validate(&self) -> bool {
        match self {
            FlashingCustomization::LinuxSdSysconfig(sd_customization)
            | FlashingCustomization::LinuxSdCloudInit(sd_customization) => {
                sd_customization.validate_user()
            }
            FlashingCustomization::T3GemInit { config, desktop } => config.build(*desktop).is_ok(),
            _ => true,
        }
    }

    pub(crate) fn validation_error(&self, lang: gem_i18n::Lang) -> Option<&'static str> {
        match self {
            FlashingCustomization::T3GemInit { config, desktop } => {
                config.build(*desktop).err().map(|error| {
                    use gem_flasher::t3_gem_init::T3GemInitError;
                    let msg = match error {
                        T3GemInitError::ControlCharacter { .. } => {
                            gem_i18n::Msg::InvalidControlCharacter
                        }
                        T3GemInitError::InvalidHostname => gem_i18n::Msg::InvalidHostnameError,
                        T3GemInitError::InvalidWifiCountry => {
                            gem_i18n::Msg::InvalidWifiCountryError
                        }
                        T3GemInitError::InvalidSsid => gem_i18n::Msg::InvalidSsidError,
                        T3GemInitError::SsidUnsupportedByCurrentSdk => {
                            gem_i18n::Msg::SsidUnsupportedError
                        }
                        T3GemInitError::UnknownTimezone(_) => gem_i18n::Msg::UnknownTimezoneError,
                        T3GemInitError::UnknownKeyboardLayout(_) => {
                            gem_i18n::Msg::UnknownKeymapError
                        }
                        T3GemInitError::WifiPassphraseLength => {
                            gem_i18n::Msg::InvalidWifiPasswordError
                        }
                        T3GemInitError::VncPasswordTooLong { .. } => {
                            gem_i18n::Msg::VncPasswordTooLongError
                        }
                        T3GemInitError::EmptyPassword => gem_i18n::Msg::EmptyPasswordError,
                        T3GemInitError::PasswordHash | T3GemInitError::Csprng => {
                            gem_i18n::Msg::PasswordGenerationError
                        }
                    };
                    lang.text(msg)
                })
            }
            _ => None,
        }
    }

    #[cfg(feature = "sd")]
    fn sd_customization(self) -> anyhow::Result<gem_flasher::sd::FlashingSdLinuxConfig> {
        Ok(match self {
            FlashingCustomization::LinuxSdSysconfig(c) => c.sysconfig(),
            FlashingCustomization::LinuxSdCloudInit(c) => c.cloudinit(),
            FlashingCustomization::NoneSd => gem_flasher::sd::FlashingSdLinuxConfig::none(),
            FlashingCustomization::T3GemInit { config, desktop } => {
                let config = config.build(desktop)?;
                gem_flasher::sd::FlashingSdLinuxConfig::t3_gem_init(&config)?
            }
        })
    }
}

#[cfg(target_os = "linux")]
async fn show_notification_xdg_portal(body: &str) -> ashpd::Result<()> {
    let proxy = ashpd::desktop::notification::NotificationProxy::new().await?;

    proxy
        .add_notification(
            constants::APP_ID,
            ashpd::desktop::notification::Notification::new(constants::APP_NAME).body(body),
        )
        .await
}

pub(crate) async fn show_notification(body: String) -> anyhow::Result<()> {
    #[cfg(all(not(target_os = "linux"), not(feature = "notify-rust")))]
    let _ = &body;

    #[cfg(target_os = "linux")]
    if show_notification_xdg_portal(&body).await.is_ok() {
        return Ok(());
    }

    #[cfg(feature = "notify-rust")]
    if tokio::task::spawn_blocking(move || {
        notify_rust::Notification::new()
            .appname(constants::APP_NAME)
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

pub(crate) fn app_title(_: &crate::GemImager) -> String {
    if cfg!(feature = "pre-release") {
        format!("{} (pre-release)", constants::APP_NAME)
    } else {
        format!("{} v{}", constants::APP_NAME, env!("CARGO_PKG_VERSION"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OsImageId {
    Format,
    Local(config::Flasher),
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
    pub(crate) fn format() -> Self {
        Self {
            id: OsImageId::Format,
            icon: None,
            label: Cow::Borrowed(""),
        }
    }

    pub(crate) fn local(flasher: config::Flasher) -> Self {
        Self {
            id: OsImageId::Local(flasher),
            icon: None,
            label: Cow::Borrowed(""),
        }
    }

    pub(crate) const fn is_sublist(&self) -> bool {
        matches!(self.id, OsImageId::OsSublist(_))
    }

    pub(crate) fn localized_label(&self, lang: gem_i18n::Lang) -> &str {
        match self.id {
            OsImageId::Format => lang.text(gem_i18n::Msg::FormatSdCard),
            OsImageId::Local(_) => lang.text(gem_i18n::Msg::SelectLocalImage),
            OsImageId::OsImage(_) | OsImageId::OsSublist(_) => &self.label,
        }
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
    pub(crate) fn msg(&'a self) -> GemImagerMessage {
        match self {
            DestinationItem::SaveToFile(x) => {
                GemImagerMessage::SelectFileDest(normalize_file_dest(x))
            }
            DestinationItem::Destination(d) => GemImagerMessage::SelectDest((*d).clone()),
        }
    }

    pub(crate) fn is_selected(&'a self, dst: &'a Destination) -> bool {
        match self {
            DestinationItem::SaveToFile(_) => false,
            DestinationItem::Destination(d) => dst.eq(d),
        }
    }

    pub(crate) fn subtitle(&self, lang: gem_i18n::Lang) -> Option<String> {
        match self {
            DestinationItem::SaveToFile(_) => None,
            DestinationItem::Destination(d) if let Some((title, _)) = d.unavailable_reason() => {
                Some(lang.text(title).to_owned())
            }
            DestinationItem::Destination(d) if d.is_dfu() => {
                Some(lang.text(gem_i18n::Msg::DfuDestinationSubtitle).to_owned())
            }
            DestinationItem::Destination(d) => d.size().map(crate::helpers::pretty_bytes),
        }
    }
}

pub(crate) fn fetch_images(
    downloader: &gem_downloader::Downloader,
    iter: impl IntoIterator<Item = url::Url>,
) -> iced::Task<GemImagerMessage> {
    let tasks = iter.into_iter().map(|icon| {
        let downloader = downloader.clone();
        let icon_clone = icon.clone();
        let icon_clone2 = icon.clone();
        iced::Task::perform(
            async move { downloader.download(icon_clone).await },
            move |p| match p {
                Ok(p) => GemImagerMessage::ResolveImage(icon_clone2, p),
                Err(_) => {
                    tracing::warn!("Failed to fetch image {}", icon);
                    GemImagerMessage::Null
                }
            },
        )
    });

    iced::Task::batch(tasks)
}

fn is_t3_catalog(url: &Url) -> bool {
    gem_config::t3::T3_CATALOG_URL
        .parse::<Url>()
        .ok()
        .and_then(|canonical| Some((canonical.host_str()?.to_owned(), url.host_str()?)))
        .is_some_and(|(canonical_host, host)| canonical_host == host)
}

pub(crate) async fn fetch_remote_config(
    downloader: &gem_downloader::Downloader,
    url: Url,
) -> std::io::Result<gem_config::config::Config> {
    if !is_t3_catalog(&url) {
        return Ok(downloader.download_json_no_cache(url).await?);
    }

    let raw: gem_config::t3::RawT3Catalog = downloader.download_json_no_cache(url.clone()).await?;

    let parsed = gem_config::t3::validate_catalog(
        raw,
        gem_config::t3::ProductScope::T3AndBeagleY,
        url.as_str(),
    )
    .map_err(|e| std::io::Error::other(format!("T3 catalog rejected: {e}")))?;

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

    let config = gem_config::t3::catalog_to_config(&parsed.catalog);
    tracing::info!(
        "T3 catalog: {} board(s) and {} image(s) in scope",
        config.imager.devices.len(),
        config.image_count()
    );

    Ok(config)
}

pub(crate) fn fetch_remote_subitems(
    items: impl IntoIterator<Item = (i64, Url)>,
    downloader: gem_downloader::Downloader,
) -> iced::Task<GemImagerMessage> {
    let temp = items.into_iter().map(move |(id, url)| {
        let url_clone = url.clone();
        let dl = downloader.clone();
        iced::Task::perform(
            async move { dl.download_json_no_cache(url_clone).await },
            move |x| match x {
                Ok(json) => GemImagerMessage::ResolveRemoteSubitemItem {
                    item: json,
                    target: id,
                },
                Err(e) => {
                    tracing::error!("Failed to get remote item {}: {e}", url.as_str());
                    GemImagerMessage::Null
                }
            },
        )
    });

    iced::Task::batch(temp)
}

pub(crate) fn sd_modifications_common(
    x: &crate::persistance::SdSysconfCustomization,
    lang: gem_i18n::Lang,
) -> Vec<&'static str> {
    let mut ans = Vec::new();

    if x.user.is_some() {
        ans.push(lang.text(gem_i18n::Msg::UserAccountConfigured));
    }
    if x.wifi.is_some() {
        ans.push(lang.text(gem_i18n::Msg::WifiConfigured));
    }
    if x.hostname.is_some() {
        ans.push(lang.text(gem_i18n::Msg::HostnameConfigured));
    }
    if x.keymap.is_some() {
        ans.push(lang.text(gem_i18n::Msg::KeymapConfigured));
    }
    if x.timezone.is_some() {
        ans.push(lang.text(gem_i18n::Msg::TimezoneConfigured));
    }
    if x.ssh.is_some() {
        ans.push(lang.text(gem_i18n::Msg::SshKeyConfigured));
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
        T3GemInitCustomization,
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
        assert_eq!(
            flasher_supported(config::Flasher::SdCard),
            cfg!(feature = "sd")
        );
    }

    #[test]
    fn t3_validation_errors_are_actionable_in_both_languages() {
        let customization = FlashingCustomization::T3GemInit {
            config: T3GemInitCustomization::default()
                .update_hostname(Some("-invalid-hostname".into())),
            desktop: true,
        };

        let en = customization.validation_error(gem_i18n::Lang::En).unwrap();
        let tr = customization.validation_error(gem_i18n::Lang::Tr).unwrap();
        assert!(en.contains("valid hostname"));
        assert!(tr.contains("geçerli bir makine adı"));
        assert_ne!(en, tr);
    }

    #[test]
    fn sd_modifications_common_lists_configured_fields() {
        assert!(
            sd_modifications_common(&SdSysconfCustomization::default(), gem_i18n::Lang::En)
                .is_empty()
        );

        let full = SdSysconfCustomization::default()
            .update_hostname(Some("h".into()))
            .update_timezone(Some("UTC".parse().unwrap()))
            .update_keymap(Some("us".into()))
            .update_ssh(Some("k".into()))
            .update_user(Some(SdCustomizationUser::new("u".into(), "p")))
            .update_wifi(Some(SdCustomizationWifi::default()));
        let mods = sd_modifications_common(&full, gem_i18n::Lang::En);
        assert_eq!(mods.len(), 6);
        assert!(mods.contains(&gem_i18n::Lang::En.text(gem_i18n::Msg::UserAccountConfigured)));
        assert!(mods.contains(&gem_i18n::Lang::En.text(gem_i18n::Msg::WifiConfigured)));
        assert!(mods.contains(&gem_i18n::Lang::En.text(gem_i18n::Msg::SshKeyConfigured)));

        let tr = sd_modifications_common(&full, gem_i18n::Lang::Tr);
        assert!(tr.contains(&gem_i18n::Lang::Tr.text(gem_i18n::Msg::UserAccountConfigured)));
        assert_ne!(mods, tr);
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
        for invalid in ["root", "", "   "] {
            let customization = || {
                SdSysconfCustomization::default()
                    .update_user(Some(SdCustomizationUser::new(invalid.into(), "p")))
            };
            assert!(
                !FlashingCustomization::LinuxSdSysconfig(customization()).validate(),
                "sysconfig accepted invalid username {invalid:?}"
            );
            assert!(
                !FlashingCustomization::LinuxSdCloudInit(customization()).validate(),
                "cloud-init accepted invalid username {invalid:?}"
            );
        }
    }

    #[test]
    fn staged_remote_reader_closes_its_handle_before_removing_the_file() {
        let staging = crate::staging::StagingImage::create(0).unwrap();
        let path = staging.path().to_path_buf();
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        let reader = StagedRemoteImage {
            file,
            _staging: staging,
        };

        drop(reader);

        assert!(
            !path.exists(),
            "the staging guard tried to remove a still-open remote image"
        );
    }

    #[test]
    fn remote_dfu_preflight_includes_archive_cache_growth() {
        let cache = tempfile::tempdir().unwrap();
        let remote = RemoteImage::new(
            "uncached image".into(),
            "https://example.invalid/image.xz"
                .parse::<url::Url>()
                .unwrap()
                .into(),
            [7u8; 32],
            Some(7),
            None,
            11,
            gem_downloader::Downloader::new(cache.path()).unwrap(),
        );

        assert_eq!(SelectedImage::from(remote).staging_size_estimate(), 29);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn remote_resolver_finishes_every_integrity_gate_before_returning() {
        use httpmock::{Method::GET, MockServer};
        use sha2::{Digest as _, Sha256};
        use std::io::Read as _;

        let server = MockServer::start();
        let cache = tempfile::tempdir().unwrap();
        let downloader = gem_downloader::Downloader::with_policy(
            cache.path(),
            gem_downloader::TransportPolicy::plaintext_for_tests(),
        )
        .unwrap();
        let content = b"complete extracted image";
        let archive_sha256: [u8; 32] = Sha256::digest(content).into();
        let url: url::Url = server.url("/image").parse().unwrap();

        server.mock(|when, then| {
            when.method(GET).path("/image");
            then.status(200).body(content);
        });

        let wrong_archive = RemoteImage::new(
            "bad archive".into(),
            url.clone().into(),
            [0u8; 32],
            Some(content.len() as u64),
            Some(archive_sha256),
            content.len() as u64,
            downloader.clone(),
        );
        let archive_err = match tokio::task::spawn_blocking(move || {
            wrong_archive.into_image_fn(gem_helper::cancel::CancellationToken::default())()
        })
        .await
        .unwrap()
        {
            Ok(_) => panic!("archive mismatch returned a reader to the raw writer"),
            Err(error) => error,
        };
        assert!(archive_err.to_string().contains("sha256"));

        let wrong_extract = RemoteImage::new(
            "bad extract".into(),
            url.into(),
            archive_sha256,
            Some(content.len() as u64),
            Some([1u8; 32]),
            content.len() as u64,
            downloader,
        );
        let extract_err = match tokio::task::spawn_blocking(
            wrong_extract.into_image_fn(gem_helper::cancel::CancellationToken::default()),
        )
        .await
        .unwrap()
        {
            Ok(_) => panic!("extracted mismatch returned a reader to the raw writer"),
            Err(error) => error,
        };
        assert!(extract_err.to_string().contains("sha256"));

        let wrong_legacy_size = RemoteImage::new(
            "bad legacy size".into(),
            server.url("/image").parse::<url::Url>().unwrap().into(),
            archive_sha256,
            Some(content.len() as u64),
            None,
            content.len() as u64 + 1,
            gem_downloader::Downloader::with_policy(
                cache.path(),
                gem_downloader::TransportPolicy::plaintext_for_tests(),
            )
            .unwrap(),
        );
        let size_err = match tokio::task::spawn_blocking(
            wrong_legacy_size.into_image_fn(gem_helper::cancel::CancellationToken::default()),
        )
        .await
        .unwrap()
        {
            Ok(_) => panic!("legacy size mismatch returned a reader to the raw writer"),
            Err(error) => error,
        };
        assert!(size_err.to_string().contains("size mismatch"));

        let oversized_legacy = RemoteImage::new(
            "oversized legacy image".into(),
            server.url("/image").parse::<url::Url>().unwrap().into(),
            archive_sha256,
            Some(content.len() as u64),
            None,
            content.len() as u64 - 1,
            gem_downloader::Downloader::with_policy(
                cache.path(),
                gem_downloader::TransportPolicy::plaintext_for_tests(),
            )
            .unwrap(),
        );
        let oversized_err = match tokio::task::spawn_blocking(
            oversized_legacy.into_image_fn(gem_helper::cancel::CancellationToken::default()),
        )
        .await
        .unwrap()
        {
            Ok(_) => panic!("oversized legacy image returned a reader to the raw writer"),
            Err(error) => error,
        };
        assert!(oversized_err.to_string().contains("size mismatch"));

        let valid = RemoteImage::new(
            "valid image".into(),
            server.url("/image").parse::<url::Url>().unwrap().into(),
            archive_sha256,
            Some(content.len() as u64),
            Some(archive_sha256),
            content.len() as u64,
            gem_downloader::Downloader::with_policy(
                cache.path(),
                gem_downloader::TransportPolicy::plaintext_for_tests(),
            )
            .unwrap(),
        );
        let (mut reader, size) = tokio::task::spawn_blocking(
            valid.into_image_fn(gem_helper::cancel::CancellationToken::default()),
        )
        .await
        .unwrap()
        .expect("matching archive and extracted hashes must return a reader");
        let mut actual = Vec::new();
        reader.read_to_end(&mut actual).unwrap();
        assert_eq!(size, content.len() as u64);
        assert_eq!(actual, content);
        drop(reader);

        let cancelled = RemoteImage::new(
            "cancelled image".into(),
            server.url("/image").parse::<url::Url>().unwrap().into(),
            archive_sha256,
            Some(content.len() as u64),
            Some(archive_sha256),
            content.len() as u64,
            gem_downloader::Downloader::with_policy(
                cache.path(),
                gem_downloader::TransportPolicy::plaintext_for_tests(),
            )
            .unwrap(),
        );
        let token = gem_helper::cancel::CancellationToken::default();
        drop(token.drop_guard());
        let cancelled_err = match tokio::task::spawn_blocking(cancelled.into_image_fn(token))
            .await
            .unwrap()
        {
            Ok(_) => panic!("cancelled resolver returned a reader to the raw writer"),
            Err(error) => error,
        };
        assert_eq!(cancelled_err.kind(), io::ErrorKind::Interrupted);
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
        assert!(item.subtitle(gem_i18n::Lang::En).is_none());
        match item.msg() {
            GemImagerMessage::SelectFileDest(name) => assert_eq!(name, "os.img"),
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
        assert!(item.subtitle(gem_i18n::Lang::En).is_none());
    }

    fn board(name: &str, emmc_dfu: bool) -> crate::db::Board {
        crate::db::Board {
            id: 1,
            name: name.to_string(),
            icon: None,
            tags: Vec::new(),
            description: String::new(),
            documentation: None,
            specification: Vec::new(),
            oshw: None,
            flasher: config::Flasher::SdCard,
            emmc_dfu,
            instructions: None,
        }
    }

    fn catalog_image() -> BoardImage {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"0123456789").unwrap();
        BoardImage::local(file.path().to_path_buf(), config::Flasher::SdCard)
    }

    #[test]
    fn a_dfu_capable_board_offers_both_write_methods() {
        let methods = WriteMethods::resolve(&board("T3-GEM-O1", true), &catalog_image());

        assert_eq!(methods.sd, cfg!(feature = "sd"));
        assert_eq!(methods.dfu, cfg!(feature = "dfu"));
        assert!(!methods.is_empty() || (!cfg!(feature = "sd") && !cfg!(feature = "dfu")));
    }

    #[test]
    fn a_board_without_the_capability_is_never_offered_dfu() {
        let methods = WriteMethods::resolve(&board("BeagleY-AI", false), &catalog_image());

        assert!(!methods.dfu);
        assert_eq!(methods.sd, cfg!(feature = "sd"));
    }

    #[test]
    fn formatting_a_card_is_never_a_dfu_operation() {
        let methods = WriteMethods::resolve(&board("T3-GEM-O1", true), &BoardImage::format());

        assert!(!methods.dfu);
        assert!(!BoardImage::format().supports_dfu());
        assert!(catalog_image().supports_dfu());
    }

    #[test]
    fn a_destination_that_disappeared_is_deselected() {
        let present = Destination::LocalFile(PathBuf::from("/tmp/present.img"));
        let gone = Destination::LocalFile(PathBuf::from("/tmp/gone.img"));

        assert_eq!(
            keep_selected_destination(Some(gone.clone()), std::slice::from_ref(&present)),
            Some(gone)
        );
        assert_eq!(
            keep_selected_destination(Some(present.clone()), std::slice::from_ref(&present)),
            Some(present)
        );
        assert_eq!(keep_selected_destination(None, &[]), None);
    }

    #[cfg(feature = "dfu")]
    #[test]
    fn an_unplugged_dfu_board_is_deselected() {
        let Some(dest) = destinations(
            WriteMethods {
                sd: false,
                dfu: true,
            },
            true,
        )
        .into_iter()
        .next() else {
            return;
        };

        assert!(dest.is_dfu());
        assert_eq!(
            keep_selected_destination(Some(dest.clone()), std::slice::from_ref(&dest)),
            Some(dest.clone())
        );
        assert_eq!(keep_selected_destination(Some(dest), &[]), None);
    }

    #[test]
    fn os_image_item_constructors_and_predicates() {
        let local = OsImageItem::local(config::Flasher::SdCard);
        assert_eq!(local.id, OsImageId::Local(config::Flasher::SdCard));
        assert!(!local.is_sublist());
        assert_eq!(
            local.localized_label(gem_i18n::Lang::En),
            "Select Local Image"
        );
        assert_eq!(local.localized_label(gem_i18n::Lang::Tr), "Yerel imaj seç");

        let format = OsImageItem::format();
        assert_eq!(format.id, OsImageId::Format);
        assert!(!format.is_sublist());
        assert_eq!(format.localized_label(gem_i18n::Lang::En), "Format SD Card");
        assert_eq!(
            format.localized_label(gem_i18n::Lang::Tr),
            "SD kartı biçimlendir"
        );
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
        assert_eq!(image.localized_label(gem_i18n::Lang::En), "Debian");

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
        assert!(!system_keymap().is_empty());
    }
}
