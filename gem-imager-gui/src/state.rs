use std::time::{Duration, Instant};

use gem_config::config;
use iced::{Task, widget};

use crate::{
    GemImager, constants,
    db::{self, Board},
    helpers::{self, DestinationItem, OsImageId, OsImageItem, blocking_future},
    message::GemImagerMessage,
    persistance, updater,
};

#[derive(Debug)]
pub(crate) struct GemImagerCommon {
    pub(crate) app_config: persistance::GuiConfiguration,
    pub(crate) downloader: gem_downloader::Downloader,
    pub(crate) timezones: widget::combo_box::State<chrono_tz::Tz>,
    pub(crate) keymaps: widget::combo_box::State<&'static str>,

    pub(crate) img_handle_cache: gem_iced_widgets::cached_icon::Cache<url::Url>,

    pub(crate) scroll_id: widget::Id,
    pub(crate) db: db::Db,

    /// The language every screen renders in.
    ///
    /// Resolved once at start-up from the stored preference, then the system locale, then the
    /// default — and held here rather than looked up per view so a language change is a single
    /// state transition instead of a cache that can go stale mid-flow.
    pub(crate) lang: gem_i18n::Lang,

    /// Per-attachment WinUSB offer state. The probe itself is read-only; mutation is delegated
    /// to the separately elevated helper.
    #[cfg(feature = "dfu-driver-mvp")]
    pub(crate) dfu_driver: crate::driver_ui::DfuDriverUiState,
}

impl GemImagerCommon {
    /// The language every screen renders in.
    pub(crate) fn lang(&self) -> gem_i18n::Lang {
        self.lang
    }

    /// Switch language and remember the choice.
    ///
    /// The persisted write is best-effort: failing to save a language preference must not
    /// interrupt a flash in progress, so it is logged rather than surfaced.
    pub(crate) fn set_lang(&mut self, lang: gem_i18n::Lang) {
        self.lang = lang;
        self.app_config = self.app_config.clone().update_language(lang);

        if let Err(e) = self.app_config.save() {
            tracing::error!("Failed to persist the language preference: {e}");
        }
    }

    pub(crate) fn updater_task(&self) -> Task<GemImagerMessage> {
        if cfg!(feature = "updater") {
            let downloader = self.downloader.clone();
            Task::perform(
                async move { updater::check_update(downloader).await },
                |x| match x {
                    Ok(Some(ver)) => GemImagerMessage::UpdateAvailable(ver),
                    Ok(None) => {
                        tracing::info!("Application is at the latest version");
                        GemImagerMessage::Null
                    }
                    Err(e) => {
                        tracing::error!("Failed to check for application update: {e:?}");
                        GemImagerMessage::Null
                    }
                },
            )
        } else {
            Task::none()
        }
    }

    pub(crate) fn fetch_board_images(&self) -> Task<GemImagerMessage> {
        let db = self.db.clone();
        Task::perform(
            blocking_future(move || db.board_icons().unwrap()),
            GemImagerMessage::FilterResolveImages,
        )
    }
}

#[derive(Debug)]
pub(crate) struct ChooseBoardState {
    pub(crate) common: GemImagerCommon,
    pub(crate) boards: Vec<db::BoardListItem>,
    pub(crate) selected_board: Option<Board>,
    pub(crate) search_text: String,
}

impl ChooseBoardState {
    pub(crate) fn refresh_board_list(&self) -> Task<GemImagerMessage> {
        let db = self.common.db.clone();
        let search = self.search_text.clone();

        Task::perform(
            blocking_future(move || db.board_list(&search).unwrap()),
            GemImagerMessage::UpdateBoardList,
        )
    }

    pub(crate) fn update_search(&mut self, search: String) -> Task<GemImagerMessage> {
        self.search_text = search;
        self.refresh_board_list()
    }
}

impl From<ChooseOsState> for ChooseBoardState {
    fn from(value: ChooseOsState) -> Self {
        Self {
            common: value.common,
            boards: Vec::new(),
            selected_board: Some(value.selected_board),
            search_text: String::new(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct ChooseOsState {
    pub(crate) common: GemImagerCommon,
    pub(crate) selected_board: Board,
    pub(crate) images: Vec<OsImageItem>,
    pub(crate) pos: Option<i64>,
    pub(crate) flasher: config::Flasher,
    pub(crate) selected_image: Option<(OsImageId, helpers::BoardImage)>,
    pub(crate) search_text: String,
}

impl ChooseOsState {
    pub(crate) fn update_images(&mut self, mut imgs: Vec<OsImageItem>, pos: Option<i64>) {
        // `Flasher` only has `SdCard` now, so every board offers the format and
        // local-image entries.
        imgs.extend([
            OsImageItem::format(),
            OsImageItem::local(config::Flasher::SdCard),
        ]);

        self.images = imgs;
        self.pos = pos;
    }

    pub(crate) fn img_json(&self) -> Option<String> {
        self.selected_image
            .as_ref()
            .map(|(_, b)| serde_json::to_string_pretty(&b).unwrap())
    }

    pub(crate) fn resolve_remote_sublists(
        &self,
        board_id: i64,
        pos: Option<i64>,
    ) -> Task<GemImagerMessage> {
        let db = self.common.db.clone();
        let downloader = self.common.downloader.clone();

        Task::future(blocking_future(move || {
            db.os_remote_sublists(board_id, pos).unwrap()
        }))
        .then(move |items| helpers::fetch_remote_subitems(items, downloader.clone()))
    }

    pub(crate) fn resolve_all_remote_sublists(&self, board_id: i64) -> Task<GemImagerMessage> {
        let db = self.common.db.clone();
        let downloader = self.common.downloader.clone();

        Task::future(blocking_future(move || {
            db.os_remote_sublists_by_board(board_id).unwrap()
        }))
        .then(move |items| helpers::fetch_remote_subitems(items, downloader.clone()))
    }

    pub(crate) fn refresh_image_list(&self) -> Task<GemImagerMessage> {
        let db = self.common.db.clone();
        let pos = self.pos;
        let board_id = self.selected_board.id;

        if self.search_text.is_empty() {
            Task::perform(
                blocking_future(move || {
                    let imgs = db.os_image_items(board_id, pos).unwrap();
                    (imgs, pos)
                }),
                GemImagerMessage::UpdateOsList,
            )
        } else {
            let search = self.search_text.clone();
            Task::perform(
                blocking_future(move || {
                    let imgs = db.os_images_by_name(board_id, &search).unwrap();
                    (imgs, pos)
                }),
                GemImagerMessage::UpdateOsList,
            )
        }
    }

    pub(crate) fn update_search(&mut self, search: String) -> Task<GemImagerMessage> {
        self.search_text = search;
        self.refresh_image_list()
    }

    pub fn update_pos(
        &mut self,
        pos: Option<i64>,
        flasher: config::Flasher,
    ) -> Task<GemImagerMessage> {
        self.pos = pos;
        self.flasher = flasher;
        self.refresh_image_list()
    }
}

impl From<CustomizeState> for ChooseOsState {
    fn from(value: CustomizeState) -> Self {
        ChooseDestState::from(value).into()
    }
}

impl From<ChooseDestState> for ChooseOsState {
    fn from(value: ChooseDestState) -> Self {
        Self {
            common: value.common,
            images: Vec::new(),
            flasher: value.selected_board.flasher,
            selected_board: value.selected_board,
            pos: None,
            selected_image: Some(value.selected_image),
            search_text: String::new(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct ChooseDestState {
    pub(crate) common: GemImagerCommon,
    pub(crate) selected_board: Board,
    pub(crate) selected_image: (OsImageId, helpers::BoardImage),
    pub(crate) selected_dest: Option<helpers::Destination>,
    pub(crate) destinations: Vec<helpers::Destination>,
    pub(crate) filter_destination: bool,
    pub(crate) search_text: String,
    /// Which write methods this board/image pair allows.
    ///
    /// Resolved once when the screen is entered rather than re-derived per frame, so the list, the
    /// enumeration subscription and the instructions can never disagree about whether DFU is on
    /// offer.
    pub(crate) write_methods: helpers::WriteMethods,
    /// Whether the "board is not in DFU mode" notice is open.
    ///
    /// Kept per screen rather than on [`GemImagerCommon`], which is carried verbatim through every
    /// `From` conversion as well as `restart()` and `back()` — a modal parked there would follow
    /// the user all the way through Customize, Review and Flashing.
    pub(crate) dfu_notice: bool,
}

impl ChooseDestState {
    /// Whether to show the placeholder row standing in for an absent DFU target.
    ///
    /// All three conditions are load-bearing:
    /// - `write_methods.dfu`: on a board/image pair that cannot use DFU the row must never appear.
    /// - `search_text.is_empty()`: the search filter is applied inside the enumeration
    ///   subscription, so while a search is active an empty `destinations` says nothing about
    ///   whether a board is attached.
    /// - no real DFU target listed: one or more real rows suppress the placeholder.
    pub(crate) fn show_dfu_placeholder(&self) -> bool {
        show_dfu_placeholder(
            self.write_methods.dfu,
            &self.search_text,
            self.destinations.iter().any(|d| d.is_dfu()),
        )
    }

    pub(crate) fn destinations<'a>(&'a self) -> impl Iterator<Item = DestinationItem<'a>> + 'a {
        let iter = self.destinations.iter().map(DestinationItem::Destination);

        let temp = match self.selected_image.1.file_name() {
            Some(x) => vec![DestinationItem::SaveToFile(x)],
            None => vec![],
        };

        iter.chain(temp)
    }

    pub(crate) fn instruction(&self) -> Option<&str> {
        match self.selected_image.1.info_text() {
            Some(x) => Some(x),
            None => self.selected_board.instructions.as_deref(),
        }
    }

    pub(crate) fn update_search(&mut self, search: String) {
        self.search_text = search;
    }
}

/// The placeholder-row predicate, split out from [`ChooseDestState`] so it can be exercised
/// without standing up a board, an image and a full common state around it.
fn show_dfu_placeholder(
    board_supports_dfu: bool,
    search_text: &str,
    a_real_dfu_target_is_listed: bool,
) -> bool {
    board_supports_dfu && search_text.is_empty() && !a_real_dfu_target_is_listed
}

impl From<CustomizeState> for ChooseDestState {
    fn from(value: CustomizeState) -> Self {
        // Recomputed rather than carried along: going BACK is also how a user reaches this screen
        // after changing the image, and the write methods are a property of the pair.
        let write_methods =
            helpers::WriteMethods::resolve(&value.selected_board, &value.selected_image.1);

        Self {
            common: value.common,
            selected_board: value.selected_board,
            selected_image: value.selected_image,
            selected_dest: Some(value.selected_dest),
            destinations: Vec::new(),
            filter_destination: true,
            search_text: String::new(),
            write_methods,
            // Arriving here via BACK must not resurrect a notice the user already dismissed.
            dfu_notice: false,
        }
    }
}

#[derive(Debug)]
pub(crate) struct CustomizeState {
    pub(crate) common: GemImagerCommon,
    pub(crate) selected_board: Board,
    pub(crate) selected_image: (OsImageId, helpers::BoardImage),
    pub(crate) selected_dest: helpers::Destination,
    pub(crate) customization: helpers::FlashingCustomization,
    /// Whether the review page is displaying the final destructive-action confirmation.
    pub(crate) erase_confirmation: bool,
}

impl CustomizeState {
    pub(crate) fn save_app_config(&self) -> Task<GemImagerMessage> {
        let config = self.common.app_config.clone();
        Task::future(blocking_future(move || {
            if let Err(e) = config.save() {
                tracing::error!("Failed to save config: {e}");
            }
            GemImagerMessage::Null
        }))
    }

    pub(crate) fn selected_destination(&self) -> String {
        match self.selected_dest.size() {
            Some(x) => format!("{} ({})", self.selected_dest, helpers::pretty_bytes(x)),
            None => self.selected_dest.to_string(),
        }
    }

    pub(crate) fn is_download(&self) -> bool {
        self.selected_dest.is_download_action()
    }

    pub(crate) fn modifications(&self) -> Vec<&'static str> {
        match &self.customization {
            helpers::FlashingCustomization::LinuxSdSysconfig(x) => {
                let mut ans = helpers::sd_modifications_common(x, self.common.lang());
                if x.usb_enable_dhcp == Some(true) {
                    ans.push(self.common.lang().text(gem_i18n::Msg::UsbDhcpEnabled));
                }

                ans
            }
            helpers::FlashingCustomization::LinuxSdCloudInit(x) => {
                helpers::sd_modifications_common(x, self.common.lang())
            }
            _ => Vec::new(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct FlashingState {
    pub(crate) common: GemImagerCommon,
    pub(crate) selected_board: Board,
    pub(crate) cancel_flashing: iced::task::Handle,
    pub(crate) progress: gem_flasher::DownloadFlashingStatus,
    pub(crate) start_timestamp: Option<Instant>,
    pub(crate) is_download: bool,
    pub(crate) selected_image: (OsImageId, helpers::BoardImage),
    pub(crate) selected_dest: helpers::Destination,
    pub(crate) customization: helpers::FlashingCustomization,
    /// Highest overall fraction reached so far.
    ///
    /// The write is a sequence of passes and each pass reports its own 0..1. Displaying those
    /// directly makes the indicator fall back towards zero every time one pass hands over to the
    /// next — which reads as "it started over", not as "it moved on". The screen therefore renders
    /// one monotonic axis, and this is its high-water mark.
    pub(crate) max_progress: f32,
}

impl FlashingState {
    pub(crate) fn time_remaining(&self) -> Option<Duration> {
        time_remaining_from(self.progress, self.start_timestamp.map(|t| t.elapsed()))
    }

    pub(crate) fn progress_update(&mut self, u: gem_flasher::DownloadFlashingStatus) {
        // Required for better time estimate.
        match u {
            gem_flasher::DownloadFlashingStatus::DownloadingProgress(_)
            | gem_flasher::DownloadFlashingStatus::FlashingProgress(_)
            | gem_flasher::DownloadFlashingStatus::RawWrite(_)
                if self.start_timestamp.is_none() =>
            {
                self.start_timestamp = Some(Instant::now())
            }
            _ => {}
        }

        if let Some(fraction) = flash_phase(u, self.selected_dest.is_dfu()).fraction {
            self.max_progress = self.max_progress.max(fraction);
        }
        self.progress = u;
    }

    /// What to draw right now: the label, and the fraction — or `None` while a phase with nothing
    /// to count is running.
    pub(crate) fn phase(&self) -> FlashPhase {
        let phase = flash_phase(self.progress, self.selected_dest.is_dfu());
        FlashPhase {
            fraction: phase.fraction.map(|x| x.max(self.max_progress)),
            ..phase
        }
    }
}

/// Where a status sits on the overall axis, and what it is called.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FlashPhase {
    pub(crate) label: gem_i18n::Msg,
    /// `None` for phases whose duration cannot be measured — the board re-enumerating, or the eMMC
    /// flush after the last byte. Those get an indeterminate indicator rather than an invented
    /// number that would sit still and then jump.
    pub(crate) fraction: Option<f32>,
}

/// Place a status on the single overall axis.
///
/// The two flows have different phase sets and very different cost distributions, so the weights
/// are per flow. The DFU numbers follow `instruction.md` §13.4: the raw eMMC stream dominates, and
/// the three boot artifacts — three orders of magnitude smaller — must not take a quarter of the
/// bar each.
pub(crate) fn flash_phase(status: gem_flasher::DownloadFlashingStatus, is_dfu: bool) -> FlashPhase {
    use gem_flasher::DownloadFlashingStatus as S;
    use gem_i18n::Msg;

    let span = |base: f32, width: f32, x: f32| Some(base + width * x.clamp(0.0, 1.0));

    match (status, is_dfu) {
        (S::Preparing, _) => FlashPhase {
            label: Msg::Preparing,
            fraction: Some(0.0),
        },

        // ---- SD: download, write, read back, customize ---------------------------------------
        (S::DownloadingProgress(x), false) => FlashPhase {
            label: Msg::Downloading,
            fraction: span(0.0, 0.50, x),
        },
        (S::FlashingProgress(x), false) => FlashPhase {
            label: Msg::FlashingImage,
            fraction: span(0.50, 0.35, x),
        },
        (S::Verifying(x), false) => FlashPhase {
            label: Msg::VerifyingWrittenData,
            fraction: span(0.85, 0.12, x),
        },
        (S::Customizing, false) => FlashPhase {
            label: Msg::Customizing,
            fraction: Some(0.98),
        },

        // ---- DFU: the same four passes against a staging file, then the board -----------------
        (S::DownloadingProgress(x), true) => FlashPhase {
            label: Msg::Downloading,
            fraction: span(0.0, 0.30, x),
        },
        (S::FlashingProgress(x), true) => FlashPhase {
            label: Msg::PreparingImage,
            fraction: span(0.30, 0.15, x),
        },
        (S::Verifying(x), true) => FlashPhase {
            label: Msg::VerifyingWrittenData,
            fraction: span(0.45, 0.07, x),
        },
        (S::Customizing, true) => FlashPhase {
            label: Msg::Customizing,
            fraction: Some(0.53),
        },
        (S::ResolvingBootArtifacts, _) => FlashPhase {
            label: Msg::ResolvingBootArtifacts,
            fraction: None,
        },
        // Reading the staged image end to end sits between the boot files and the first USB
        // packet. It is measurable and slow, so it gets its own slice rather than hiding inside
        // the indeterminate phase before it.
        (S::ChecksummingImage(x), _) => FlashPhase {
            label: Msg::ChecksummingImage,
            fraction: span(0.53, 0.03, x),
        },
        (S::Reconnecting, _) => FlashPhase {
            label: Msg::WaitingForBoard,
            fraction: None,
        },
        (S::BootStage { stage, progress }, _) => FlashPhase {
            label: Msg::WritingBootloader,
            // Three stages sharing 6 % of the axis: visible movement, honest weight.
            fraction: span(
                0.56 + f32::from(stage.saturating_sub(1)) * 0.02,
                0.02,
                progress,
            ),
        },
        (S::RawWrite(x), _) => FlashPhase {
            label: Msg::WritingToEmmc,
            fraction: span(0.62, 0.33, x),
        },
        (S::Finalizing, _) => FlashPhase {
            label: Msg::FinalizingWrite,
            fraction: None,
        },
    }
}

/// Estimate the remaining flashing time from the current `progress` and how
/// much time has `elapsed` since the first progress update.
///
/// Split out of [`FlashingState::time_remaining`] so the ETA math is testable
/// without an `Instant` clock: a linear extrapolation `elapsed * (1 - x) / x`,
/// suppressed until progress clears a small threshold to avoid wild early
/// estimates.
fn time_remaining_from(
    progress: gem_flasher::DownloadFlashingStatus,
    elapsed: Option<Duration>,
) -> Option<Duration> {
    const THRESHOLD: f32 = 0.02;

    match progress {
        gem_flasher::DownloadFlashingStatus::FlashingProgress(x)
        | gem_flasher::DownloadFlashingStatus::DownloadingProgress(x) => {
            if x < THRESHOLD {
                None
            } else {
                let t = elapsed?;
                let x = x.clamp(0.0, 1.0);
                let scale = (1.0 - x) / x;
                Some(t.mul_f32(scale))
            }
        }
        gem_flasher::DownloadFlashingStatus::Customizing => Some(Duration::from_secs(1)),
        _ => None,
    }
}

#[derive(Debug)]
pub(crate) struct FlashingFinishState {
    pub(crate) common: GemImagerCommon,
    pub(crate) selected_board: Board,
    pub(crate) is_download: bool,
    /// Whether the write that just ended went over DFU.
    ///
    /// Recorded here because `selected_dest` does not survive the conversion, and because
    /// `!is_download` is not a stand-in: an SD card write is not a download either.
    pub(crate) is_dfu: bool,
    /// Whether the user has closed the "switch back to eMMC" notice.
    ///
    /// The notice is open on entry, so there is no separate "show" flag; visibility is
    /// `is_dfu && !notice_dismissed`.
    pub(crate) notice_dismissed: bool,
}

impl From<FlashingState> for FlashingFinishState {
    fn from(value: FlashingState) -> Self {
        Self {
            common: value.common,
            selected_board: value.selected_board,
            is_download: value.is_download,
            is_dfu: value.selected_dest.is_dfu(),
            notice_dismissed: false,
        }
    }
}

pub(crate) struct FlashingFailState {
    pub(crate) common: GemImagerCommon,
    pub(crate) err: String,
    pub(crate) logs: widget::text_editor::Content,
    pub(crate) selected_board: Board,
    pub(crate) selected_image: (OsImageId, helpers::BoardImage),
    pub(crate) selected_dest: helpers::Destination,
    pub(crate) customization: helpers::FlashingCustomization,
}

impl From<FlashingFailState> for CustomizeState {
    fn from(value: FlashingFailState) -> Self {
        Self {
            common: value.common,
            selected_board: value.selected_board,
            selected_image: value.selected_image,
            selected_dest: value.selected_dest,
            customization: value.customization,
            erase_confirmation: false,
        }
    }
}

// State for Pages that can be opened from any of the normal pages but are not part of normal flow.
// Eg: Application info
pub(crate) enum OverlayData {
    ChooseBoard(ChooseBoardState),
    ChooseOs(ChooseOsState),
    ChooseDest(ChooseDestState),
    Customize(CustomizeState),
    Review(CustomizeState),
    Flashing(FlashingState),
    FlashingCancel(FlashingFinishState),
    FlashingFail(FlashingFailState),
    FlashingSuccess(FlashingFinishState),
}

impl OverlayData {
    pub(crate) fn common_mut(&mut self) -> &mut GemImagerCommon {
        match self {
            Self::ChooseBoard(x) => &mut x.common,
            Self::ChooseOs(x) => &mut x.common,
            Self::ChooseDest(x) => &mut x.common,
            Self::Customize(x) => &mut x.common,
            Self::Review(x) => &mut x.common,
            Self::Flashing(x) => &mut x.common,
            Self::FlashingCancel(x) => &mut x.common,
            Self::FlashingFail(x) => &mut x.common,
            Self::FlashingSuccess(x) => &mut x.common,
        }
    }

    pub(crate) fn common(&self) -> &GemImagerCommon {
        match self {
            Self::ChooseBoard(x) => &x.common,
            Self::ChooseOs(x) => &x.common,
            Self::ChooseDest(x) => &x.common,
            Self::Customize(x) => &x.common,
            Self::Review(x) => &x.common,
            Self::Flashing(x) => &x.common,
            Self::FlashingCancel(x) => &x.common,
            Self::FlashingFail(x) => &x.common,
            Self::FlashingSuccess(x) => &x.common,
        }
    }
}

impl TryFrom<GemImager> for OverlayData {
    type Error = ();

    fn try_from(value: GemImager) -> Result<Self, Self::Error> {
        match value {
            GemImager::ChooseBoard(x) => Ok(Self::ChooseBoard(x)),
            GemImager::ChooseOs(x) => Ok(Self::ChooseOs(x)),
            GemImager::ChooseDest(x) => Ok(Self::ChooseDest(x)),
            GemImager::Customize(x) => Ok(Self::Customize(x)),
            GemImager::Review(x) => Ok(Self::Review(x)),
            GemImager::Flashing(x) => Ok(Self::Flashing(x)),
            GemImager::FlashingCancel(x) => Ok(Self::FlashingCancel(x)),
            GemImager::FlashingFail(x) => Ok(Self::FlashingFail(x)),
            GemImager::FlashingSuccess(x) => Ok(Self::FlashingSuccess(x)),
            GemImager::Dummy | GemImager::AppInfo(_) => Err(()),
        }
    }
}

impl From<OverlayData> for GemImager {
    fn from(value: OverlayData) -> Self {
        match value {
            OverlayData::ChooseBoard(x) => Self::ChooseBoard(x),
            OverlayData::ChooseOs(x) => Self::ChooseOs(x),
            OverlayData::ChooseDest(x) => Self::ChooseDest(x),
            OverlayData::Customize(x) => Self::Customize(x),
            OverlayData::Review(x) => Self::Review(x),
            OverlayData::Flashing(x) => Self::Flashing(x),
            OverlayData::FlashingCancel(x) => Self::FlashingCancel(x),
            OverlayData::FlashingFail(x) => Self::FlashingFail(x),
            OverlayData::FlashingSuccess(x) => Self::FlashingSuccess(x),
        }
    }
}

pub(crate) struct OverlayState {
    pub(crate) page: OverlayData,
    pub(crate) log_path: String,
    pub(crate) license: widget::text_editor::Content,
    pub(crate) cache_dir: String,
}

impl OverlayState {
    pub(crate) fn new(page: OverlayData) -> Self {
        let log_path = helpers::log_file_path().to_string_lossy().to_string();
        let license = widget::text_editor::Content::with_text(constants::APP_LINCESE);
        let cache_dir = helpers::project_dirs()
            .unwrap()
            .cache_dir()
            .to_string_lossy()
            .to_string();

        Self {
            page,
            log_path,
            license,
            cache_dir,
        }
    }

    pub(crate) fn common(&self) -> &GemImagerCommon {
        self.page.common()
    }

    pub(crate) fn common_mut(&mut self) -> &mut GemImagerCommon {
        self.page.common_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::{flash_phase, show_dfu_placeholder, time_remaining_from};
    use gem_flasher::DownloadFlashingStatus;
    use std::time::Duration;

    #[test]
    fn placeholder_hidden_when_board_lacks_dfu_support() {
        assert!(!show_dfu_placeholder(false, "", false));
    }

    /// Regression lock. The search filter is applied inside the enumeration subscription, so while
    /// a search is active `destinations` is empty for reasons that have nothing to do with whether
    /// a board is attached. Without this condition the notice would claim "not connected" about a
    /// board that is physically plugged in.
    #[test]
    fn placeholder_hidden_while_searching() {
        assert!(!show_dfu_placeholder(true, "sd", false));
    }

    #[test]
    fn placeholder_hidden_when_a_real_dfu_target_is_listed() {
        assert!(!show_dfu_placeholder(true, "", true));
    }

    #[test]
    fn placeholder_shown_when_dfu_supported_and_nothing_enumerated() {
        assert!(show_dfu_placeholder(true, "", false));
    }

    /// The whole point of the shared axis: every phase of a DFU write, in the order the backend
    /// emits them, must produce a non-decreasing fraction. Before this, staging ended near the top
    /// of the bar and the DFU transfer restarted it at zero.
    #[test]
    fn a_dfu_write_never_moves_the_indicator_backwards() {
        let sequence = [
            DownloadFlashingStatus::Preparing,
            DownloadFlashingStatus::DownloadingProgress(0.5),
            DownloadFlashingStatus::DownloadingProgress(1.0),
            DownloadFlashingStatus::FlashingProgress(0.0),
            DownloadFlashingStatus::FlashingProgress(1.0),
            DownloadFlashingStatus::Verifying(0.0),
            DownloadFlashingStatus::Verifying(1.0),
            DownloadFlashingStatus::Customizing,
            DownloadFlashingStatus::ResolvingBootArtifacts,
            DownloadFlashingStatus::ChecksummingImage(0.0),
            DownloadFlashingStatus::ChecksummingImage(1.0),
            DownloadFlashingStatus::Reconnecting,
            DownloadFlashingStatus::BootStage {
                stage: 1,
                progress: 0.0,
            },
            DownloadFlashingStatus::BootStage {
                stage: 3,
                progress: 1.0,
            },
            DownloadFlashingStatus::RawWrite(0.0),
            DownloadFlashingStatus::RawWrite(1.0),
            DownloadFlashingStatus::Finalizing,
        ];

        let mut last = 0.0_f32;
        for status in sequence {
            // Unmeasurable phases report no fraction; they must not reset the axis either.
            let Some(fraction) = flash_phase(status, true).fraction else {
                continue;
            };
            // The epsilon covers f32 representation at a hand-over point (0.30 + 0.15 lands a
            // fraction of a ulp above 0.45), not a real regression; `max_progress` clamps the
            // rendered value anyway.
            assert!(
                fraction >= last - f32::EPSILON,
                "{status:?} moved the indicator from {last} to {fraction}"
            );
            last = fraction;
        }
        assert!(last > 0.9, "the axis should be nearly full by the end");
        assert!(
            last < 1.0,
            "the measurable work must not claim success before finalization"
        );
    }

    /// The same rule on the SD flow, where the read-back pass used to restart the bar.
    #[test]
    fn an_sd_write_never_moves_the_indicator_backwards() {
        let sequence = [
            DownloadFlashingStatus::DownloadingProgress(1.0),
            DownloadFlashingStatus::FlashingProgress(0.0),
            DownloadFlashingStatus::FlashingProgress(1.0),
            DownloadFlashingStatus::Verifying(0.0),
            DownloadFlashingStatus::Verifying(1.0),
            DownloadFlashingStatus::Customizing,
        ];

        let mut last = 0.0_f32;
        for status in sequence {
            let fraction = flash_phase(status, false).fraction.unwrap();
            assert!(fraction >= last, "{status:?} moved {last} -> {fraction}");
            last = fraction;
        }
    }

    /// The three boot artifacts are three orders of magnitude smaller than the raw image, so they
    /// must not occupy a comparable share of the axis.
    #[test]
    fn the_raw_emmc_stream_dominates_the_axis() {
        let boot_share = flash_phase(
            DownloadFlashingStatus::BootStage {
                stage: 3,
                progress: 1.0,
            },
            true,
        )
        .fraction
        .unwrap()
            - flash_phase(
                DownloadFlashingStatus::BootStage {
                    stage: 1,
                    progress: 0.0,
                },
                true,
            )
            .fraction
            .unwrap();
        let raw_share = flash_phase(DownloadFlashingStatus::RawWrite(1.0), true)
            .fraction
            .unwrap()
            - flash_phase(DownloadFlashingStatus::RawWrite(0.0), true)
                .fraction
                .unwrap();

        assert!(
            raw_share > boot_share * 4.0,
            "raw {raw_share} vs boot {boot_share}"
        );
    }

    /// Phases the backend cannot measure must say so rather than inventing a number.
    #[test]
    fn unmeasurable_phases_are_indeterminate() {
        for status in [
            DownloadFlashingStatus::ResolvingBootArtifacts,
            DownloadFlashingStatus::Reconnecting,
            DownloadFlashingStatus::Finalizing,
        ] {
            assert!(flash_phase(status, true).fraction.is_none());
        }
    }

    #[test]
    fn eta_scales_linearly_with_remaining_fraction() {
        // At 50% after 10s, the remaining half should take another ~10s.
        assert_eq!(
            time_remaining_from(
                DownloadFlashingStatus::FlashingProgress(0.5),
                Some(Duration::from_secs(10)),
            ),
            Some(Duration::from_secs(10))
        );
        // At 25% after 10s, the remaining 75% extrapolates to 30s.
        assert_eq!(
            time_remaining_from(
                DownloadFlashingStatus::FlashingProgress(0.25),
                Some(Duration::from_secs(10)),
            ),
            Some(Duration::from_secs(30))
        );
    }

    #[test]
    fn eta_uses_the_same_math_for_downloads() {
        assert_eq!(
            time_remaining_from(
                DownloadFlashingStatus::DownloadingProgress(0.5),
                Some(Duration::from_secs(4)),
            ),
            Some(Duration::from_secs(4))
        );
    }

    #[test]
    fn eta_suppressed_below_threshold() {
        // Below 2% the estimate is too noisy, so no ETA is reported.
        assert_eq!(
            time_remaining_from(
                DownloadFlashingStatus::FlashingProgress(0.01),
                Some(Duration::from_secs(10)),
            ),
            None
        );
    }

    #[test]
    fn eta_requires_a_start_timestamp() {
        // Past the threshold but with no elapsed time recorded yet.
        assert_eq!(
            time_remaining_from(DownloadFlashingStatus::FlashingProgress(0.5), None),
            None
        );
    }

    #[test]
    fn eta_clamps_progress_above_one() {
        // A progress value >1.0 clamps to 1.0, yielding a zero remainder.
        assert_eq!(
            time_remaining_from(
                DownloadFlashingStatus::FlashingProgress(1.5),
                Some(Duration::from_secs(10)),
            ),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn customizing_reports_fixed_estimate() {
        assert_eq!(
            time_remaining_from(DownloadFlashingStatus::Customizing, None),
            Some(Duration::from_secs(1))
        );
    }

    #[test]
    fn non_progress_states_have_no_eta() {
        assert_eq!(
            time_remaining_from(
                DownloadFlashingStatus::Preparing,
                Some(Duration::from_secs(5))
            ),
            None
        );
        assert_eq!(
            time_remaining_from(
                DownloadFlashingStatus::Verifying(0.5),
                Some(Duration::from_secs(5))
            ),
            None
        );
    }
}
