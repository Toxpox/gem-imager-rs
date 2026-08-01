//! Layer 3 of the T3 catalog adapter: the canonical model the rest of the application uses.
//!
//! Nothing here is deserialized directly from the network. Values only reach these types through
//! [`crate::t3::validate`], which is where the the catalog validation rules invariants are enforced.

use std::collections::BTreeSet;
use std::time::Duration;

use url::Url;

use crate::t3::sha256::Sha256;

/// Board tag of the mandatory target board (the T3 platform contract).
pub const T3_BOARD_TAG: &str = "t3-gem-o1";

/// Board tag of BeagleY-AI in the live T3 catalog.
pub const BEAGLEY_BOARD_TAG: &str = "beagley-ai";

/// USB vendor id the T3 board enumerates with in DFU mode (the T3 platform contract).
pub const T3_DFU_VENDOR_ID: u16 = 0x0451;

/// USB product id the T3 board enumerates with in DFU mode (the T3 platform contract).
pub const T3_DFU_PRODUCT_ID: u16 = 0x6165;

/// Boot manifest for the T3 board (the T3 platform contract).
pub const T3_BOOT_MANIFEST_URL: &str =
    "https://packages.t3gemstone.org/images/boot/t3-gem-o1/list.json";

/// Upper bound on waiting for a DFU alt-setting to re-enumerate.
///
/// The working reference retries device discovery at most 15 times at 1s intervals
/// (the T3 DFU contract).
pub const T3_DFU_RECONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// DFU alt-setting the raw eMMC image is written to (the T3 DFU contract, stage 4).
pub const T3_RAW_EMMC_ALT_SETTING: &str = "rawemmc";

/// How a board filters the image list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MatchingType {
    /// Only images explicitly tagged for this board are shown.
    #[default]
    Exclusive,
    /// Images tagged for this board are shown alongside untagged ones.
    Inclusive,
}

impl MatchingType {
    /// Parse the catalog's `matching_type` string.
    ///
    /// Unknown values fall back to the stricter [`MatchingType::Exclusive`] so a future value
    /// cannot silently widen what a board is offered.
    pub fn parse(raw: &str) -> Self {
        match raw {
            "inclusive" => Self::Inclusive,
            _ => Self::Exclusive,
        }
    }
}

/// First-boot customization consumer declared by an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum T3InitFormat {
    /// The T3 GemInit `config.ini` consumer shipped by `gem-first-boot`.
    Systemd,
}

impl T3InitFormat {
    /// Parse the catalog's `init_format` string, or `None` if this build has no consumer for it.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "systemd" => Some(Self::Systemd),
            _ => None,
        }
    }
}

/// What customization an image supports.
///
/// the supported first-boot contract restricts the writable field set to what the current `gem-first-boot`
/// consumer actually reads; this type only records *which* consumer applies, never the values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomizationProfile {
    /// Which first-boot consumer this image ships.
    pub init_format: T3InitFormat,
    /// Whether this is a desktop variant, which the supported first-boot contract ties to VNC support.
    ///
    /// The T3 catalog exposes no explicit desktop flag, so this is derived from the image name.
    /// It gates *offering* the VNC fields only; This must stay aligned with the real
    /// `gem-first-boot` consumer before those fields are written.
    pub desktop_variant: bool,
}

/// One stage of the T3 DFU boot chain.
///
/// `artifact_name` and `alt_setting` are separate fields and must never be conflated
/// (the catalog validation rules) — for `tiboot3.bin` they genuinely differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DfuStageSpec {
    /// File name as published in the boot manifest.
    pub artifact_name: String,
    /// DFU alt-setting this artifact is written to.
    pub alt_setting: String,
    /// Whether the device resets and re-enumerates after this stage.
    pub reset_after: bool,
    /// How long to wait for the next alt-setting to appear.
    pub reconnect_timeout: Duration,
}

/// Everything needed to drive a USB DFU write for a board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DfuProfile {
    /// USB vendor id in DFU mode.
    pub vendor_id: u16,
    /// USB product id in DFU mode.
    pub product_id: u16,
    /// Where the board's boot artifact manifest lives.
    pub boot_manifest: Url,
    /// Boot artifact stages, in mandatory order.
    pub stages: Vec<DfuStageSpec>,
    /// Alt-setting the extracted OS image is streamed to after the boot chain completes.
    pub raw_emmc_alt_setting: String,
}

impl DfuProfile {
    /// The verified profile for T3-GEM-O1.
    ///
    /// The boot manifest only publishes `name` and `sha256`; alt-settings, ordering and reset
    /// behaviour come from the working-reference contract in the T3 DFU contract and are
    /// therefore compile-time constants, not server-supplied data.
    pub fn t3_gem_o1() -> Self {
        let stages = [
            ("tiboot3.bin", "bootloader"),
            ("tispl.bin", "tispl.bin"),
            ("u-boot.img", "u-boot.img"),
        ]
        .into_iter()
        .map(|(artifact_name, alt_setting)| DfuStageSpec {
            artifact_name: artifact_name.to_owned(),
            alt_setting: alt_setting.to_owned(),
            reset_after: true,
            reconnect_timeout: T3_DFU_RECONNECT_TIMEOUT,
        })
        .collect();

        Self {
            vendor_id: T3_DFU_VENDOR_ID,
            product_id: T3_DFU_PRODUCT_ID,
            // Parsing a compile-time constant that is covered by a unit test below.
            boot_manifest: Url::parse(T3_BOOT_MANIFEST_URL)
                .expect("T3_BOOT_MANIFEST_URL is a valid URL"),
            stages,
            raw_emmc_alt_setting: T3_RAW_EMMC_ALT_SETTING.to_owned(),
        }
    }

    /// Artifact names the boot manifest must publish, in order.
    pub fn required_artifacts(&self) -> Vec<&str> {
        self.stages
            .iter()
            .map(|stage| stage.artifact_name.as_str())
            .collect()
    }
}

/// What a board can be written through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardCapabilities {
    /// Whether the board boots from a removable SD card.
    pub sd: bool,
    /// Present only when a *verified* DFU profile exists for this board.
    pub emmc_dfu: Option<DfuProfile>,
}

impl BoardCapabilities {
    /// SD-only board.
    pub fn sd_only() -> Self {
        Self {
            sd: true,
            emmc_dfu: None,
        }
    }

    /// Whether DFU is available.
    pub fn supports_dfu(&self) -> bool {
        self.emmc_dfu.is_some()
    }
}

/// A way of getting an image onto a board.
///
/// the target selection rules forbids treating DFU as an image's single flasher: the same T3 image
/// supports both destinations, so the write method is a property of the board/image pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WriteMethod {
    /// Write to a removable SD card.
    Sd,
    /// Write to onboard eMMC over USB DFU.
    EmmcDfu,
}

/// A board in the canonical model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Board {
    /// Display name.
    pub name: String,
    /// Human description.
    pub description: String,
    /// Tags images refer to.
    pub tags: BTreeSet<String>,
    /// Board icon.
    pub icon: Option<Url>,
    /// Write capabilities.
    pub capabilities: BoardCapabilities,
    /// How this board filters the image list.
    pub matching_type: MatchingType,
    /// Whether the board is offered by default.
    pub is_default: bool,
    /// Whether the board is inside the configured product scope.
    ///
    /// Out-of-scope boards stay in the model so a later scope change needs no re-parse
    /// (the T3 platform contract); the product surface must filter on this flag.
    pub in_product_scope: bool,
}

impl Board {
    /// Whether this board is the mandatory T3 target.
    pub fn is_t3(&self) -> bool {
        self.tags.iter().any(|tag| tag == T3_BOARD_TAG)
    }

    /// Whether an image declares support for this board.
    pub fn accepts(&self, image: &Image) -> bool {
        image.devices.iter().any(|tag| self.tags.contains(tag))
    }

    /// Write methods available for this board/image pair.
    ///
    /// This covers the first two factors of the the target selection rules intersection — board
    /// capability and image/board compatibility. Callers must still intersect the result with
    /// platform backend availability and physically attached targets.
    pub fn write_methods_for(&self, image: &Image) -> Vec<WriteMethod> {
        if !self.accepts(image) {
            return Vec::new();
        }

        let mut methods = Vec::new();
        if self.capabilities.sd {
            methods.push(WriteMethod::Sd);
        }
        if self.capabilities.supports_dfu() {
            methods.push(WriteMethod::EmmcDfu);
        }
        methods
    }
}

/// The four independent integrity gates for an image (the integrity policy).
///
/// Archive and extracted values are separate fields with distinct names so one can never stand in
/// for the other (the catalog validation rules).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageIntegrity {
    /// SHA-256 of the compressed archive as downloaded.
    pub archive_sha256: Sha256,
    /// Size of the compressed archive, when the catalog publishes it.
    pub archive_size: Option<u64>,
    /// SHA-256 of the image after extraction.
    pub extracted_sha256: Sha256,
    /// Size of the image after extraction. Always present and always non-zero.
    pub extracted_size: u64,
}

/// An image in the canonical model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// Display name.
    pub name: String,
    /// Human description.
    pub description: String,
    /// Image icon.
    pub icon: Option<Url>,
    /// Download URL of the compressed archive.
    pub url: Url,
    /// The four integrity gates.
    pub integrity: ImageIntegrity,
    /// Release date.
    pub release_date: chrono::NaiveDate,
    /// Board tags this image supports.
    pub devices: BTreeSet<String>,
    /// Customization support, or `None` when no consumer in this build handles the image.
    pub customization: Option<CustomizationProfile>,
    /// Sub-list this image was published under, if any.
    pub group: Option<String>,
}

impl Image {
    /// Whether this image targets the mandatory T3 board.
    pub fn is_t3(&self) -> bool {
        self.devices.iter().any(|tag| tag == T3_BOARD_TAG)
    }
}

/// Which boards the product surface exposes (the product scope policy, ADR 0001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProductScope {
    /// Only T3-GEM-O1 is offered. The safe default.
    #[default]
    T3Only,
    /// T3-GEM-O1 with SD+DFU, plus BeagleY-AI as SD-only.
    T3AndBeagleY,
}

impl ProductScope {
    /// Whether a board carrying these tags is inside the product surface.
    pub fn includes(&self, tags: &BTreeSet<String>) -> bool {
        match self {
            Self::T3Only => tags.iter().any(|tag| tag == T3_BOARD_TAG),
            Self::T3AndBeagleY => tags
                .iter()
                .any(|tag| tag == T3_BOARD_TAG || tag == BEAGLEY_BOARD_TAG),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    #[test]
    fn t3_boot_manifest_url_constant_is_a_valid_url() {
        let profile = DfuProfile::t3_gem_o1();
        assert_eq!(profile.boot_manifest.scheme(), "https");
    }

    #[test]
    fn t3_profile_matches_the_documented_stage_contract() {
        let profile = DfuProfile::t3_gem_o1();
        assert_eq!(profile.vendor_id, 0x0451);
        assert_eq!(profile.product_id, 0x6165);
        assert_eq!(
            profile.required_artifacts(),
            ["tiboot3.bin", "tispl.bin", "u-boot.img"]
        );
        // tiboot3 is the one stage whose alt-setting differs from its artifact name.
        assert_eq!(profile.stages[0].alt_setting, "bootloader");
        assert_ne!(
            profile.stages[0].alt_setting,
            profile.stages[0].artifact_name
        );
        assert_eq!(profile.raw_emmc_alt_setting, "rawemmc");
        assert!(profile.stages.iter().all(|stage| stage.reset_after));
    }

    #[test]
    fn unknown_matching_type_falls_back_to_exclusive() {
        assert_eq!(MatchingType::parse("inclusive"), MatchingType::Inclusive);
        assert_eq!(MatchingType::parse("exclusive"), MatchingType::Exclusive);
        assert_eq!(
            MatchingType::parse("something-new"),
            MatchingType::Exclusive
        );
    }

    #[test]
    fn unknown_init_format_has_no_consumer() {
        assert_eq!(T3InitFormat::parse("systemd"), Some(T3InitFormat::Systemd));
        assert_eq!(T3InitFormat::parse("cloudinit"), None);
    }

    #[test]
    fn t3_only_scope_excludes_beagley() {
        let scope = ProductScope::T3Only;
        assert!(scope.includes(&tags(&["t3-gem-o1"])));
        assert!(!scope.includes(&tags(&["beagley-ai"])));
    }

    #[test]
    fn combined_scope_includes_both_boards() {
        let scope = ProductScope::T3AndBeagleY;
        assert!(scope.includes(&tags(&["t3-gem-o1"])));
        assert!(scope.includes(&tags(&["beagley-ai"])));
        assert!(!scope.includes(&tags(&["some-other-board"])));
    }

    #[test]
    fn sd_only_board_never_offers_dfu() {
        let capabilities = BoardCapabilities::sd_only();
        assert!(capabilities.sd);
        assert!(!capabilities.supports_dfu());
    }
}
