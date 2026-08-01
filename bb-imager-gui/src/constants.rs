use iced::color;

pub(crate) const OSHW_BASE_URL: &str = "https://certification.oshwa.org";

/// Updater endpoint for the T3 Gemstone fork.
/// The updater is feature-gated (`updater`) and is not enabled in the M1 preview packages.
pub(crate) const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/t3gemstone/imager/releases/latest";

/// The canonical application identity, reversed-domain style.
///
/// Deliberately **not** `org.t3gemstone.gem-imager`: the Qt reference application owns that
/// identity, and a stable install of it must keep its own config/cache/data directories. This
/// tuple is what `directories::ProjectDirs` derives those paths from, so sharing it would let
/// the two applications overwrite each other's state.
pub(crate) const PACKAGE_QUALIFIER: (&str, &str, &str) = ("org", "t3gemstone", "imager");

/// The same identity flattened, for surfaces that want one string: the XDG notification
/// application id and `mac_notification_sys`/`notify-rust`.
///
/// Windows names the application through the embedded executable manifest and the AppX identity
/// instead, so nothing reads this there.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) const APP_ID: &str = "org.t3gemstone.imager";

pub(crate) const DEFAULT_CONFIG: &[u8] = include_bytes!("../../config.json");
pub(crate) const WINDOW_SIZE: iced::Size = iced::Size::new(680.0, 450.0);
pub(crate) const APP_NAME: &str = "T3 Gemstone Imager";
pub(crate) const APP_RELEASE: &str = if cfg!(feature = "pre-release") {
    "pre-release"
} else {
    env!("CARGO_PKG_VERSION")
};
pub(crate) const APP_DESC: &str = env!("CARGO_PKG_DESCRIPTION");
pub(crate) const APP_LINCESE: &str = include_str!("../../LICENSE");

// Icons
pub(crate) const WINDOW_ICON_BYTES: &[u8] = include_bytes!("../assets/icons/icon.png");
pub(crate) const ARROW_BACK_ICON_BYTES: &[u8] = include_bytes!("../assets/icons/arrow-back.svg");
pub(crate) const FILE_ADD_ICON_BYTES: &[u8] = include_bytes!("../assets/icons/file-add.svg");
pub(crate) const USB_ICON_BYTES: &[u8] = include_bytes!("../assets/icons/usb.svg");
pub(crate) const FORMAT_ICON_BYTES: &[u8] = include_bytes!("../assets/icons/format.svg");
pub(crate) const BOARD_ICON_BYTES: &[u8] = include_bytes!("../assets/icons/board.svg");
pub(crate) const ARROW_FORWARD_IOS_ICON_BYTES: &[u8] =
    include_bytes!("../assets/icons/arrow-forward-ios.svg");
pub(crate) const FILE_SAVE_ICON_BYTES: &[u8] = include_bytes!("../assets/icons/file-save.svg");
pub(crate) const INFO_ICON_BYTES: &[u8] = include_bytes!("../assets/icons/info.svg");
pub(crate) const COPY_ICON_BYTES: &[u8] = include_bytes!("../assets/icons/content-copy.svg");
pub(crate) const SEARCH_ICON_BYTES: &[u8] = include_bytes!("../assets/icons/search.svg");

// Font
pub(crate) const FONT_REGULAR: iced::Font = iced::Font::with_name("Nunito");
pub(crate) const FONT_BOLD: iced::Font = {
    let mut font = FONT_REGULAR;
    font.weight = iced::font::Weight::Bold;

    font
};

// Base Fonts
pub(crate) const FONT_NORMAL_BYTES: &[u8] =
    include_bytes!("../assets/fonts/Nunito-Regular-subset.ttf");
pub(crate) const FONT_BOLD_BYTES: &[u8] = include_bytes!("../assets/fonts/Nunito-Bold-subset.ttf");

// Theme
//
// The brand colours are taken from the T3 Gemstone reference application (`gem-imager`), so the
// two utilities look like the same product: `#18224f` is its main surface (`src/main.qml:95`,
// `qmlcomponents/ImButton.qml:17`) and `#d15d7d` its action accent (`src/main.qml:250`). The
// previous names encoded BeagleBoard's mascot (tongue orange, hair light brown) and are gone.
//
// `SUCCESS_GREEN` and `DANGER_RED` are semantic, not brand: they keep the upstream values so the
// "finished" and "this destroys data" signals do not shift meaning along with the repaint.

/// Brand accent — buttons, progress, selection.
pub(crate) const GEMSTONE_ROSE: iced::Color = color!(0xd1, 0x5d, 0x7d);
/// Brand surface — the window background.
pub(crate) const GEMSTONE_NAVY: iced::Color = color!(0x18, 0x22, 0x4f);
/// Brand surface, one step lighter — cards sitting on the background.
pub(crate) const GEMSTONE_NAVY_CARD: iced::Color = color!(0x23, 0x2e, 0x63);
pub(crate) const SUCCESS_GREEN: iced::Color = color!(142, 201, 105);
pub(crate) const WARNING_AMBER: iced::Color = color!(0xe0, 0xa3, 0x3e);
pub(crate) const DANGER_RED: iced::Color = color!(255, 0, 0);

pub(crate) const KEYMAP_LAYOUTS: &[&str] = &[
    "af", "al", "am", "ara", "at", "au", "az", "ba", "bd", "be", "bg", "br", "brai", "bt", "bw",
    "by", "ca", "cd", "ch", "cm", "cn", "cz", "de", "dk", "dz", "ee", "epo", "es", "et", "fi",
    "fo", "fr", "gb", "ge", "gh", "gn", "gr", "hr", "hu", "id", "ie", "il", "in", "iq", "ir", "is",
    "it", "jp", "jv", "ke", "kg", "kh", "kr", "kz", "la", "latam", "lk", "lt", "lv", "ma", "mao",
    "md", "me", "mk", "ml", "mm", "mn", "mt", "mv", "my", "ng", "nl", "no", "np", "ph", "pk", "pl",
    "pt", "ro", "rs", "ru", "se", "si", "sk", "sn", "sy", "tg", "th", "tj", "tm", "tr", "tw", "tz",
    "ua", "us", "uz", "vn", "za",
];

#[cfg(test)]
mod tests {
    use super::KEYMAP_LAYOUTS;

    /// The keymap combo box looks up its selection with `binary_search`, so new
    /// entries need to be inserted in byte order.
    #[test]
    fn keymap_layouts_sorted() {
        assert!(KEYMAP_LAYOUTS.is_sorted());
    }
}
