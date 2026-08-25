use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RawT3Catalog {
    #[serde(default)]
    pub imager: RawImager,
    #[serde(default)]
    pub os_list: Vec<RawOsListItem>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RawImager {
    #[serde(default)]
    pub devices: Vec<RawDevice>,
    pub latest_version: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawDevice {
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub icon: Option<String>,
    #[serde(default)]
    pub emmc: bool,
    pub matching_type: Option<String>,
    #[serde(default = "default_true", rename = "default")]
    pub is_default: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RawOsListItem {
    pub name: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub subitems: Option<Vec<RawOsListItem>>,
    pub url: Option<String>,
    pub image_download_size: Option<u64>,
    pub image_download_sha256: Option<String>,
    pub extract_size: Option<u64>,
    pub extract_sha256: Option<String>,
    pub release_date: Option<String>,
    #[serde(default)]
    pub devices: Vec<String>,
    pub init_format: Option<String>,
}

impl RawOsListItem {
    pub fn is_sublist(&self) -> bool {
        self.subitems.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_live_shaped_device_entry() {
        let json = r#"{
            "description": "T3 Gemstone Obsidian",
            "emmc": true,
            "icon": "https://packages.t3gemstone.org/images/icons/gemstone-o1-fritzing.svg",
            "matching_type": "exclusive",
            "name": "T3-GEM-O1",
            "tags": ["t3-gem-o1"]
        }"#;

        let device: RawDevice = serde_json::from_str(json).expect("device parses");
        assert_eq!(device.name.as_deref(), Some("T3-GEM-O1"));
        assert!(device.emmc);
        assert_eq!(device.tags, ["t3-gem-o1"]);
        assert!(device.is_default);
    }

    #[test]
    fn respects_an_explicit_default_false() {
        let json = r#"{"name": "No filtering", "tags": [], "default": false}"#;
        let device: RawDevice = serde_json::from_str(json).expect("device parses");
        assert!(!device.is_default);
    }

    #[test]
    fn unknown_fields_are_ignored_for_forward_compatibility() {
        let json = r#"{"name": "Pardus Images", "random": true, "subitems": []}"#;
        let item: RawOsListItem = serde_json::from_str(json).expect("item parses");
        assert!(item.is_sublist());
    }

    #[test]
    fn missing_required_image_fields_parse_as_none_rather_than_failing() {
        let json = r#"{"name": "Broken", "devices": ["t3-gem-o1"]}"#;
        let item: RawOsListItem = serde_json::from_str(json).expect("item parses");
        assert!(!item.is_sublist());
        assert!(item.extract_sha256.is_none());
        assert!(item.image_download_sha256.is_none());
    }

    #[test]
    fn an_empty_document_parses_into_an_empty_catalog() {
        let catalog: RawT3Catalog = serde_json::from_str("{}").expect("empty object parses");
        assert!(catalog.os_list.is_empty());
        assert!(catalog.imager.devices.is_empty());
    }
}
