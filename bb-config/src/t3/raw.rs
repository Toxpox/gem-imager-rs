//! Layer 1 of the T3 catalog adapter: serde types that mirror the server schema.
//!
//! These types deliberately do almost no validation. Every field that validation cares about is
//! `Option`, so a malformed entry produces a *typed diagnostic with a JSON path* in layer 2
//! rather than a serde error that aborts the whole document or, worse, a `VecSkipError` that drops
//! it silently (the catalog validation rules).
//!
//! Unknown fields are ignored, which is serde's default and is what the catalog validation rules asks for
//! ("Bilinmeyen gelecekteki alanlar ileri uyumluluk için yok sayılabilir"). The live catalog
//! already carries such a field (`random` on sub-list wrappers).
//!
//! Schema captured from <https://packages.t3gemstone.org/images/list.json> on 2026-07-31.

use serde::Deserialize;

/// Root of `images/list.json`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RawT3Catalog {
    /// Board list and imager metadata.
    #[serde(default)]
    pub imager: RawImager,
    /// Image tree. Entries are either images or single-level sub-lists.
    #[serde(default)]
    pub os_list: Vec<RawOsListItem>,
}

/// The `imager` object.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RawImager {
    /// Board definitions.
    #[serde(default)]
    pub devices: Vec<RawDevice>,
    /// Latest published imager version, advisory only.
    pub latest_version: Option<String>,
    /// Product landing page, advisory only.
    pub url: Option<String>,
}

/// One board definition under `imager.devices`.
///
/// Note there is no `flasher` field in the T3 schema: write capability is derived from `emmc`
/// plus the compile-time T3 DFU profile, never from a single catalog-supplied flasher enum
/// (the target selection rules).
#[derive(Debug, Clone, Deserialize)]
pub struct RawDevice {
    /// Display name, e.g. `T3-GEM-O1`.
    pub name: Option<String>,
    /// Human description.
    pub description: Option<String>,
    /// Tags an image's `devices` array refers to.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Board icon URL.
    pub icon: Option<String>,
    /// Whether the board exposes an eMMC that can be written over USB DFU.
    #[serde(default)]
    pub emmc: bool,
    /// `inclusive` or `exclusive`; controls how the board filters the image list.
    pub matching_type: Option<String>,
    /// Whether the board is offered by default. Absent means `true` in the live catalog.
    #[serde(default = "default_true", rename = "default")]
    pub is_default: bool,
}

fn default_true() -> bool {
    true
}

/// One entry in `os_list`, or in a sub-list's `subitems`.
///
/// The T3 schema uses the same array for image entries and sub-list wrappers, distinguished only
/// by the presence of `subitems`. Modelling both in one struct (instead of an untagged enum) keeps
/// error reporting precise: an image missing a required field reports *that field*, rather than
/// serde reporting "data did not match any variant".
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RawOsListItem {
    /// Image or sub-list name.
    pub name: Option<String>,
    /// Human description.
    pub description: Option<String>,
    /// Icon URL.
    pub icon: Option<String>,
    /// Present only on sub-list wrappers.
    pub subitems: Option<Vec<RawOsListItem>>,
    /// Download URL of the compressed image.
    pub url: Option<String>,
    /// Size of the compressed archive in bytes.
    pub image_download_size: Option<u64>,
    /// SHA-256 of the compressed archive, as 64 hex characters.
    pub image_download_sha256: Option<String>,
    /// Size of the image after extraction, in bytes.
    pub extract_size: Option<u64>,
    /// SHA-256 of the image after extraction, as 64 hex characters.
    pub extract_sha256: Option<String>,
    /// ISO-8601 release date.
    pub release_date: Option<String>,
    /// Board tags this image can be written to.
    #[serde(default)]
    pub devices: Vec<String>,
    /// First-boot customization consumer, e.g. `systemd`.
    pub init_format: Option<String>,
}

impl RawOsListItem {
    /// Whether this entry is a sub-list wrapper rather than an image.
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
        // Absent `default` means the board is offered.
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
        // `random` really does appear on live sub-list wrappers.
        let json = r#"{"name": "Pardus Images", "random": true, "subitems": []}"#;
        let item: RawOsListItem = serde_json::from_str(json).expect("item parses");
        assert!(item.is_sublist());
    }

    #[test]
    fn missing_required_image_fields_parse_as_none_rather_than_failing() {
        // Layer 2 turns these into diagnostics; layer 1 must not abort the document.
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
