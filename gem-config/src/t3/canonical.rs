
use std::collections::BTreeSet;
use std::time::Duration;

use url::Url;

use crate::t3::sha256::Sha256;

pub const T3_BOARD_TAG: &str = "t3-gem-o1";

pub const BEAGLEY_BOARD_TAG: &str = "beagley-ai";

pub const T3_DFU_VENDOR_ID: u16 = 0x0451;

pub const T3_DFU_PRODUCT_ID: u16 = 0x6165;

pub const T3_BOOT_MANIFEST_URL: &str =
    "https://packages.t3gemstone.org/images/boot/t3-gem-o1/list.json";

pub const T3_DFU_RECONNECT_TIMEOUT: Duration = Duration::from_secs(15);

pub const T3_RAW_EMMC_ALT_SETTING: &str = "rawemmc";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MatchingType {
    #[default]
    Exclusive,
    Inclusive,
}

impl MatchingType {
    pub fn parse(raw: &str) -> Self {
        match raw {
            "inclusive" => Self::Inclusive,
            _ => Self::Exclusive,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum T3InitFormat {
    Systemd,
}

impl T3InitFormat {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "systemd" => Some(Self::Systemd),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomizationProfile {
    pub init_format: T3InitFormat,
    pub desktop_variant: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DfuStageSpec {
    pub artifact_name: String,
    pub alt_setting: String,
    pub reset_after: bool,
    pub reconnect_timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DfuProfile {
    pub vendor_id: u16,
    pub product_id: u16,
    pub boot_manifest: Url,
    pub stages: Vec<DfuStageSpec>,
    pub raw_emmc_alt_setting: String,
}

impl DfuProfile {
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
            boot_manifest: Url::parse(T3_BOOT_MANIFEST_URL)
                .expect("T3_BOOT_MANIFEST_URL is a valid URL"),
            stages,
            raw_emmc_alt_setting: T3_RAW_EMMC_ALT_SETTING.to_owned(),
        }
    }

    pub fn required_artifacts(&self) -> Vec<&str> {
        self.stages
            .iter()
            .map(|stage| stage.artifact_name.as_str())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardCapabilities {
    pub sd: bool,
    pub emmc_dfu: Option<DfuProfile>,
}

impl BoardCapabilities {
    pub fn sd_only() -> Self {
        Self {
            sd: true,
            emmc_dfu: None,
        }
    }

    pub fn supports_dfu(&self) -> bool {
        self.emmc_dfu.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WriteMethod {
    Sd,
    EmmcDfu,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Board {
    pub name: String,
    pub description: String,
    pub tags: BTreeSet<String>,
    pub icon: Option<Url>,
    pub capabilities: BoardCapabilities,
    pub matching_type: MatchingType,
    pub is_default: bool,
    pub in_product_scope: bool,
}

impl Board {
    pub fn is_t3(&self) -> bool {
        self.tags.iter().any(|tag| tag == T3_BOARD_TAG)
    }

    pub fn accepts(&self, image: &Image) -> bool {
        image.devices.iter().any(|tag| self.tags.contains(tag))
    }

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageIntegrity {
    pub archive_sha256: Sha256,
    pub archive_size: Option<u64>,
    pub extracted_sha256: Sha256,
    pub extracted_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub name: String,
    pub description: String,
    pub icon: Option<Url>,
    pub url: Url,
    pub integrity: ImageIntegrity,
    pub release_date: chrono::NaiveDate,
    pub devices: BTreeSet<String>,
    pub customization: Option<CustomizationProfile>,
    pub group: Option<String>,
}

impl Image {
    pub fn is_t3(&self) -> bool {
        self.devices.iter().any(|tag| tag == T3_BOARD_TAG)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProductScope {
    #[default]
    T3Only,
    T3AndBeagleY,
}

impl ProductScope {
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
