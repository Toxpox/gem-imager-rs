//! Bridge from the canonical T3 model into the front-end [`crate::config`] model.
//!
//! # Why this exists
//!
//! The front-ends read [`crate::config::Config`], whose schema comes from BeagleBoard. Pointing
//! them straight at the T3 catalog does not fail loudly — it produces an **empty board and image
//! list**, because both structs are parsed with `VecSkipError` and the T3 document mismatches them
//! in two places:
//!
//! * T3 devices carry no `flasher` field, which [`crate::config::Device`] requires. Every device
//!   entry is dropped.
//! * T3 images declare `init_format: "systemd"`, which is not a [`crate::config::InitFormat`]
//!   variant. Every image entry is dropped.
//!
//! That is exactly the silent-empty-list failure the strict adapter was written to prevent. This
//! module is the missing link between the adapter and the screens: the catalog is parsed and
//! validated by [`crate::t3::validate`], and only then translated into the shape the front-ends
//! already render.
//!
//! # What is deliberately not translated
//!
//! * **Customization.** A T3 image's `systemd` consumer is the GemInit `config.ini` writer, which
//!   is not [`crate::config::InitFormat::Sysconf`] — that is BeagleBoard's `sysconf.txt`. Mapping
//!   one onto the other would make the GUI write a file the board does not read, so every bridged
//!   image reports [`crate::config::InitFormat::None`] until the T3 serializer exists.
//! * **eMMC/DFU capability.** [`crate::config::Flasher`] can only express `SdCard`. Boards keep
//!   their DFU profile in the canonical model; the bridged view is SD-only on purpose, which
//!   matches the SD-only milestone.

use std::collections::HashSet;

use crate::config::{Config, Device, Flasher, Imager, InitFormat, OsImage, OsListItem};
use crate::t3::canonical::{Board, Image};
use crate::t3::validate::ValidatedT3Catalog;

/// Translate a validated catalog into the front-end config model.
///
/// Only boards inside the configured product scope are emitted. An image is emitted only when at
/// least one emitted board accepts it, so an image can never appear under a board that did not
/// declare it.
pub fn catalog_to_config(catalog: &ValidatedT3Catalog) -> Config {
    let boards: Vec<&Board> = catalog.boards_in_scope().collect();

    let devices = boards.iter().map(|board| board_to_device(board)).collect();

    let os_list = catalog
        .images
        .iter()
        .filter(|image| boards.iter().any(|board| board.accepts(image)))
        .filter_map(|image| image_to_item(image, &boards))
        .collect();

    Config {
        imager: Imager {
            // The bridged document is terminal: it must never send the front-end back out to fetch
            // another config, or a T3 catalog could pull a BeagleBoard one back in.
            remote_configs: Vec::new(),
            devices,
        },
        os_list,
    }
}

fn board_to_device(board: &Board) -> Device {
    Device {
        name: board.name.clone(),
        tags: board.tags.iter().cloned().collect(),
        icon: board.icon.clone(),
        description: board.description.clone(),
        // See the module docs: the bridged view is SD-only by construction.
        flasher: Flasher::SdCard,
        documentation: None,
        instructions: None,
        specification: Vec::new(),
        oshw: None,
    }
}

/// Translate one image, resolving the icon the front-end requires.
///
/// [`OsImage::icon`] is mandatory while the canonical model allows it to be absent. Rather than
/// invent a URL, the icon falls back to that of a board which accepts the image. An image with no
/// icon anywhere is skipped — but loudly, never silently, because a vanishing image is the exact
/// failure this module exists to stop.
fn image_to_item(image: &Image, boards: &[&Board]) -> Option<OsListItem> {
    let icon = image.icon.clone().or_else(|| {
        boards
            .iter()
            .find(|board| board.accepts(image))
            .and_then(|board| board.icon.clone())
    });

    let Some(icon) = icon else {
        tracing::warn!(
            "Skipping T3 image \"{}\": neither the image nor any board that accepts it publishes \
             an icon, and the front-end model requires one",
            image.name
        );
        return None;
    };

    Some(OsListItem::Image(OsImage {
        name: image.name.clone(),
        description: image.description.clone(),
        icon,
        url: image.url.clone(),
        image_download_size: image.integrity.archive_size,
        image_download_sha256: *image.integrity.archive_sha256.as_bytes(),
        extract_size: image.integrity.extracted_size,
        extract_sha256: Some(*image.integrity.extracted_sha256.as_bytes()),
        release_date: image.release_date,
        devices: image.devices.iter().cloned().collect::<HashSet<_>>(),
        tags: HashSet::new(),
        // See the module docs: not Sysconf, and not silently mapped onto it.
        init_format: InitFormat::None,
        info_text: None,
        support: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::t3::canonical::ProductScope;
    use crate::t3::validate::parse_catalog;

    const SOURCE: &str = "https://packages.t3gemstone.org/images/list.json";

    /// A catalog in the live server's shape: devices without `flasher`, images with
    /// `init_format: "systemd"`.
    fn live_shaped_catalog() -> &'static str {
        r#"{
          "imager": {
            "latest_version": "1.0.0",
            "devices": [
              {
                "name": "No filtering",
                "description": "All images",
                "tags": [],
                "matching_type": "inclusive",
                "default": true
              },
              {
                "name": "T3-GEM-O1",
                "description": "T3 Gemstone board",
                "tags": ["t3-gem-o1"],
                "matching_type": "exclusive",
                "emmc": true,
                "icon": "https://packages.t3gemstone.org/images/icons/t3.svg"
              },
              {
                "name": "BeagleY-AI",
                "description": "BeagleY-AI board",
                "tags": ["beagley-ai"],
                "matching_type": "exclusive",
                "emmc": false,
                "icon": "https://packages.t3gemstone.org/images/icons/beagley.svg"
              }
            ]
          },
          "os_list": [
            {
              "name": "T3 Gemstone OS (Desktop)",
              "description": "Desktop image",
              "icon": "https://packages.t3gemstone.org/images/icons/ubuntu.svg",
              "url": "https://packages.t3gemstone.org/images/t3.img.xz",
              "image_download_size": 807607316,
              "image_download_sha256": "668a83c94264c17e9e549284b50ec1f9ec1c0a1d171ede3a92797a458eabc198",
              "extract_size": 4096000000,
              "extract_sha256": "33afbc809f8c39c4a7472c49e26f7c5ac507c5b1d97df05c42ec83e97e1f6e51",
              "release_date": "2026-03-26",
              "devices": ["t3-gem-o1"],
              "init_format": "systemd"
            },
            {
              "name": "T3 Gemstone OS For BeagleY-AI",
              "description": "BeagleY image",
              "icon": "https://packages.t3gemstone.org/images/icons/ubuntu.svg",
              "url": "https://packages.t3gemstone.org/images/beagley.img.xz",
              "image_download_size": 707607316,
              "image_download_sha256": "778a83c94264c17e9e549284b50ec1f9ec1c0a1d171ede3a92797a458eabc198",
              "extract_size": 3096000000,
              "extract_sha256": "44afbc809f8c39c4a7472c49e26f7c5ac507c5b1d97df05c42ec83e97e1f6e51",
              "release_date": "2026-03-26",
              "devices": ["beagley-ai"],
              "init_format": "systemd"
            }
          ]
        }"#
    }

    fn bridged(scope: ProductScope) -> Config {
        let parsed = parse_catalog(live_shaped_catalog().as_bytes(), scope, SOURCE)
            .expect("the live catalog shape must parse");
        catalog_to_config(&parsed.catalog)
    }

    /// The regression this module exists for: the same bytes parsed as a plain `Config` yield
    /// nothing at all, with no error to show the user.
    #[test]
    fn the_legacy_parser_silently_produces_an_empty_catalog() {
        let direct: Config = serde_json::from_str(live_shaped_catalog())
            .expect("VecSkipError makes this succeed, which is the whole problem");

        assert!(
            direct.imager.devices.is_empty(),
            "devices are dropped because the T3 schema has no `flasher` field"
        );
        assert!(
            direct.os_list.is_empty(),
            "images are dropped because `init_format: systemd` is not a legacy variant"
        );
    }

    #[test]
    fn the_bridge_exposes_the_t3_board_and_its_image() {
        let config = bridged(ProductScope::T3Only);

        let board = config
            .imager
            .devices
            .iter()
            .find(|d| d.name == "T3-GEM-O1")
            .expect("T3-GEM-O1 must reach the front-end model");
        assert!(board.tags.contains("t3-gem-o1"));

        let names: Vec<&str> = config
            .os_list
            .iter()
            .map(|item| match item {
                OsListItem::Image(img) => img.name.as_str(),
                _ => panic!("the bridge only emits plain images"),
            })
            .collect();
        assert_eq!(names, ["T3 Gemstone OS (Desktop)"]);
    }

    /// Default scope is T3-only, so BeagleY and its images must not leak into the surface.
    #[test]
    fn t3_only_scope_hides_beagley_and_its_images() {
        let config = bridged(ProductScope::T3Only);

        assert!(config.imager.devices.iter().all(|d| d.name != "BeagleY-AI"));
        assert!(config.os_list.iter().all(|item| match item {
            OsListItem::Image(img) => !img.devices.contains("beagley-ai"),
            _ => true,
        }));
    }

    #[test]
    fn combined_scope_exposes_both_boards_with_their_own_images() {
        let config = bridged(ProductScope::T3AndBeagleY);

        assert_eq!(config.imager.devices.len(), 2);
        assert_eq!(config.os_list.len(), 2);
    }

    /// Both extracted gates must survive the translation; they are what Faz 3 and Faz 4 verify
    /// against, and an image that arrives without them silently loses its integrity checking.
    #[test]
    fn both_integrity_gates_survive_the_bridge() {
        let config = bridged(ProductScope::T3Only);
        let OsListItem::Image(img) = &config.os_list[0] else {
            panic!("expected a plain image");
        };

        assert_eq!(img.extract_size, 4096000000);
        assert_eq!(
            const_hex::encode(img.extract_sha256.expect("extracted digest must survive")),
            "33afbc809f8c39c4a7472c49e26f7c5ac507c5b1d97df05c42ec83e97e1f6e51"
        );
        assert_eq!(
            const_hex::encode(img.image_download_sha256),
            "668a83c94264c17e9e549284b50ec1f9ec1c0a1d171ede3a92797a458eabc198"
        );
        assert_eq!(img.image_download_size, Some(807607316));
    }

    /// `systemd` is the GemInit consumer, not BeagleBoard's `sysconf.txt`. Until the T3 serializer
    /// exists, offering customization here would write a file the board never reads.
    #[test]
    fn customization_is_not_mapped_onto_the_beagleboard_format() {
        let config = bridged(ProductScope::T3Only);
        let OsListItem::Image(img) = &config.os_list[0] else {
            panic!("expected a plain image");
        };

        assert_eq!(img.init_format, InitFormat::None);
    }

    /// A bridged document must not send the front-end back out for another config.
    #[test]
    fn the_bridged_config_declares_no_further_remote_configs() {
        assert!(
            bridged(ProductScope::T3Only)
                .imager
                .remote_configs
                .is_empty()
        );
    }
}
