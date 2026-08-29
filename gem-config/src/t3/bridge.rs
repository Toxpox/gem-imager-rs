use std::collections::HashSet;

use url::Url;

use crate::config::{Config, Device, Flasher, Imager, InitFormat, OsImage, OsListItem, OsSubList};
use crate::t3::canonical::{Board, Image, T3_BOARD_TAG};
use crate::t3::validate::ValidatedT3Catalog;

pub fn catalog_to_config(catalog: &ValidatedT3Catalog) -> Config {
    let boards: Vec<&Board> = catalog.boards_in_scope().collect();

    let devices = boards.iter().map(|board| board_to_device(board)).collect();

    let in_scope: Vec<&Image> = catalog
        .images
        .iter()
        .filter(|image| boards.iter().any(|board| board.accepts(image)))
        .collect();

    let os_list = group_os_list(&in_scope, &boards);

    Config {
        imager: Imager {
            remote_configs: Vec::new(),
            devices,
        },
        os_list,
    }
}

const T3_GEM_O1_SPECIFICATION: &[(&str, &str)] = &[
    ("Processor", "Texas Instruments AM67A"),
    (
        "Cores",
        "4 x 1.4GHz ARM® Cortex-A53 (64-bit), 2 x 800MHz ARM® Cortex-R5F",
    ),
    ("GPU", "IMG BXS-4-64, 50 GFLOPS"),
    ("AI Accelerator", "2 x 2 TOPS deep learning accelerator"),
    ("RAM", "4GB LPDDR4"),
    ("Onboard Flash", "32GB eMMC"),
];

fn specification_for(board: &Board) -> Vec<(String, String)> {
    if !board.tags.iter().any(|tag| tag == T3_BOARD_TAG) {
        return Vec::new();
    }

    T3_GEM_O1_SPECIFICATION
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn board_to_device(board: &Board) -> Device {
    Device {
        name: board.name.clone(),
        tags: board.tags.iter().cloned().collect(),
        icon: board.icon.clone(),
        description: board.description.clone(),
        flasher: Flasher::SdCard,
        emmc_dfu: board.capabilities.supports_dfu(),
        documentation: None,
        instructions: None,
        specification: specification_for(board),
        oshw: None,
    }
}

fn init_format(image: &Image) -> InitFormat {
    match &image.customization {
        Some(profile) if profile.desktop_variant => InitFormat::GemInitDesktop,
        Some(_) => InitFormat::GemInit,
        None => InitFormat::None,
    }
}

fn image_to_item(image: &Image, boards: &[&Board]) -> Option<OsListItem> {
    let Some(icon) = resolve_icon(image, boards) else {
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
        init_format: init_format(image),
        info_text: None,
        support: None,
    }))
}

fn resolve_icon(image: &Image, boards: &[&Board]) -> Option<Url> {
    image.icon.clone().or_else(|| {
        boards
            .iter()
            .find(|board| board.accepts(image))
            .and_then(|board| board.icon.clone())
    })
}

fn group_os_list(images: &[&Image], boards: &[&Board]) -> Vec<OsListItem> {
    enum Slot<'a> {
        Loose(&'a Image),
        Distribution(usize),
    }

    let mut slots: Vec<Slot<'_>> = Vec::new();
    let mut distributions: Vec<(&str, Vec<&Image>)> = Vec::new();

    for image in images {
        let Some(group) = image.group.as_deref() else {
            slots.push(Slot::Loose(image));
            continue;
        };

        match distributions.iter().position(|(name, _)| *name == group) {
            Some(index) => distributions[index].1.push(image),
            None => {
                slots.push(Slot::Distribution(distributions.len()));
                distributions.push((group, vec![image]));
            }
        }
    }

    slots
        .into_iter()
        .filter_map(|slot| match slot {
            Slot::Loose(image) => image_to_item(image, boards),
            Slot::Distribution(index) => {
                let (name, members) = &distributions[index];
                distribution_sublist(name, members, boards)
            }
        })
        .collect()
}

fn distribution_sublist(name: &str, members: &[&Image], boards: &[&Board]) -> Option<OsListItem> {
    let Some(icon) = members.iter().find_map(|image| resolve_icon(image, boards)) else {
        tracing::warn!(
            "Skipping T3 distribution \"{name}\": no image under it publishes an icon, and the \
             front-end model requires one"
        );
        return None;
    };

    let mut releases: Vec<(String, Vec<&Image>)> = Vec::new();
    let mut undated: Vec<&Image> = Vec::new();

    for image in members {
        let Some(key) = release_key(image) else {
            tracing::warn!(
                "T3 image \"{}\" ({}) has no release segment in its URL; listing it directly under \
                 \"{name}\" rather than dropping it",
                image.name,
                image.url
            );
            undated.push(image);
            continue;
        };

        match releases.iter().position(|(existing, _)| *existing == key) {
            Some(index) => releases[index].1.push(image),
            None => releases.push((key, vec![image])),
        }
    }

    releases.sort_by_key(|release| std::cmp::Reverse(newest_release_date(&release.1)));

    let mut subitems: Vec<OsListItem> = Vec::new();

    if releases.len() > 1 {
        subitems.extend(
            releases
                .iter()
                .filter_map(|(key, images)| release_sublist(key, images, boards)),
        );
    } else if let Some((_, images)) = releases.first() {
        subitems.extend(sorted_images(images, boards));
    }

    subitems.extend(sorted_images(&undated, boards));

    if subitems.is_empty() {
        tracing::warn!("Skipping T3 distribution \"{name}\": every image under it was dropped");
        return None;
    }

    Some(OsListItem::SubList(OsSubList {
        name: name.to_owned(),
        description: releases
            .iter()
            .map(|(key, images)| release_label(key, images))
            .collect::<Vec<_>>()
            .join(", "),
        icon,
        flasher: Flasher::SdCard,
        subitems,
    }))
}

fn release_sublist(key: &str, images: &[&Image], boards: &[&Board]) -> Option<OsListItem> {
    let icon = images
        .iter()
        .find_map(|image| resolve_icon(image, boards))?;
    let subitems = sorted_images(images, boards);

    if subitems.is_empty() {
        return None;
    }

    let label = release_label(key, images);

    Some(OsListItem::SubList(OsSubList {
        name: label.clone(),
        description: label,
        icon,
        flasher: Flasher::SdCard,
        subitems,
    }))
}

fn sorted_images(images: &[&Image], boards: &[&Board]) -> Vec<OsListItem> {
    let mut ordered: Vec<&Image> = images.to_vec();
    ordered.sort_by(|a, b| {
        variant_rank(&a.name)
            .cmp(&variant_rank(&b.name))
            .then_with(|| a.name.cmp(&b.name))
    });

    ordered
        .into_iter()
        .filter_map(|image| image_to_item(image, boards))
        .collect()
}

fn release_key(image: &Image) -> Option<String> {
    let segments: Vec<&str> = image
        .url
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .collect();

    let board_position = segments
        .iter()
        .position(|segment| image.devices.contains(*segment))?;

    let release = segments.get(board_position.checked_sub(1)?)?;

    (!release.is_empty()).then(|| release.to_lowercase())
}

fn release_label(key: &str, images: &[&Image]) -> String {
    images
        .iter()
        .find_map(|image| port_target(&image.description))
        .unwrap_or_else(|| title_case(key))
}

fn port_target(description: &str) -> Option<String> {
    const PREFIX: &str = "A port of ";
    const SUFFIX: &str = " with ";

    let rest = description.strip_prefix(PREFIX)?;
    let end = rest.find(SUFFIX)?;
    let target = rest[..end].trim();

    (!target.is_empty()).then(|| target.to_owned())
}

fn title_case(key: &str) -> String {
    key.split(['-', '_'])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn newest_release_date(images: &[&Image]) -> Option<chrono::NaiveDate> {
    images.iter().map(|image| image.release_date).max()
}

fn variant_rank(name: &str) -> u8 {
    let name = name.to_lowercase();

    if name.contains("desktop") {
        0
    } else if name.contains("kiosk") {
        1
    } else if name.contains("minimal") {
        2
    } else {
        3
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::t3::canonical::ProductScope;
    use crate::t3::validate::parse_catalog;

    const SOURCE: &str = "https://packages.t3gemstone.org/images/list.json";

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

    #[test]
    fn the_t3_board_carries_its_hardware_specification() {
        let config = bridged(ProductScope::T3AndBeagleY);

        let t3 = config
            .imager
            .devices
            .iter()
            .find(|device| device.tags.contains(T3_BOARD_TAG))
            .expect("the T3 board must be bridged");

        let keys: Vec<&str> = t3
            .specification
            .iter()
            .map(|(key, _)| key.as_str())
            .collect();
        assert!(
            keys.contains(&"Processor")
                && keys.contains(&"Cores")
                && keys.contains(&"GPU")
                && keys.contains(&"RAM"),
            "the board detail pane shows these rows: {keys:?}"
        );
        assert!(
            t3.specification.iter().all(|(_, value)| !value.is_empty()),
            "an empty value renders as a blank row"
        );

        let beagley = config
            .imager
            .devices
            .iter()
            .find(|device| !device.tags.contains(T3_BOARD_TAG))
            .expect("BeagleY-AI is in scope here");
        assert!(
            beagley.specification.is_empty(),
            "only the T3 board has a specification published by this fork"
        );
    }

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

    #[test]
    fn customization_is_not_mapped_onto_the_beagleboard_formats() {
        let config = bridged(ProductScope::T3Only);

        for item in &config.os_list {
            let OsListItem::Image(img) = item else {
                panic!("expected a plain image");
            };

            assert!(
                img.init_format.is_gem_init(),
                "{} should use the T3 writer, got {:?}",
                img.name,
                img.init_format
            );
            assert_ne!(img.init_format, InitFormat::Sysconf);
            assert_ne!(img.init_format, InitFormat::CloudInit);
        }
    }

    #[test]
    fn only_desktop_images_offer_vnc() {
        let config = bridged(ProductScope::T3Only);

        for item in &config.os_list {
            let OsListItem::Image(img) = item else {
                panic!("expected a plain image");
            };

            assert_eq!(
                img.init_format.supports_vnc(),
                img.name.to_lowercase().contains("desktop"),
                "VNC availability does not match the image variant for {}",
                img.name
            );
        }
    }

    #[test]
    fn only_the_dfu_capable_board_reports_the_capability() {
        let config = bridged(ProductScope::T3AndBeagleY);

        let t3 = config
            .imager
            .devices
            .iter()
            .find(|d| d.name == "T3-GEM-O1")
            .expect("T3-GEM-O1 must reach the front-end model");
        let beagley = config
            .imager
            .devices
            .iter()
            .find(|d| d.name == "BeagleY-AI")
            .expect("BeagleY-AI must reach the front-end model in the combined scope");

        assert!(t3.emmc_dfu);
        assert!(!beagley.emmc_dfu);
    }

    #[test]
    fn the_bridged_config_declares_no_further_remote_configs() {
        assert!(
            bridged(ProductScope::T3Only)
                .imager
                .remote_configs
                .is_empty()
        );
    }

    const HASH_ARCHIVE: &str = "668a83c94264c17e9e549284b50ec1f9ec1c0a1d171ede3a92797a458eabc198";
    const HASH_EXTRACTED: &str = "33afbc809f8c39c4a7472c49e26f7c5ac507c5b1d97df05c42ec83e97e1f6e51";

    fn image_entry(name: &str, description: &str, url: &str, release_date: &str) -> String {
        format!(
            r#"{{
              "name": "{name}",
              "description": "{description}",
              "icon": "https://packages.t3gemstone.org/images/icons/os.svg",
              "url": "{url}",
              "image_download_size": 807607316,
              "image_download_sha256": "{HASH_ARCHIVE}",
              "extract_size": 4096000000,
              "extract_sha256": "{HASH_EXTRACTED}",
              "release_date": "{release_date}",
              "devices": ["t3-gem-o1"],
              "init_format": "systemd"
            }}"#
        )
    }

    fn sublist_entry(name: &str, subitems: &[String]) -> String {
        format!(
            r#"{{
              "name": "{name}",
              "description": "{name} For T3 Gemstone",
              "icon": "https://packages.t3gemstone.org/images/icons/distro.svg",
              "subitems": [{}]
            }}"#,
            subitems.join(",")
        )
    }

    fn grouped_catalog() -> String {
        let ubuntu = sublist_entry(
            "Ubuntu Images",
            &[
                image_entry(
                    "T3 Gemstone OS (Minimal)",
                    "A port of Ubuntu 22.04 (Jammy) with minimal packages",
                    "https://packages.t3gemstone.org/images/ubuntu/jammy/t3-gem-o1/minimal.img.xz",
                    "2026-03-26",
                ),
                image_entry(
                    "T3 Gemstone OS (Desktop)",
                    "A port of Ubuntu 22.04 (Jammy) with a desktop environment",
                    "https://packages.t3gemstone.org/images/ubuntu/jammy/t3-gem-o1/desktop.img.xz",
                    "2026-03-26",
                ),
                image_entry(
                    "T3 Gemstone OS (Minimal)",
                    "A port of Ubuntu 24.04 (Noble) with minimal packages",
                    "https://packages.t3gemstone.org/images/ubuntu/noble/t3-gem-o1/minimal.img.xz",
                    "2026-06-08",
                ),
                image_entry(
                    "T3 Gemstone OS (Kiosk)",
                    "A port of Ubuntu 24.04 (Noble) with a kiosk shell",
                    "https://packages.t3gemstone.org/images/ubuntu/noble/t3-gem-o1/kiosk.img.xz",
                    "2026-06-08",
                ),
                image_entry(
                    "T3 Gemstone OS (Desktop)",
                    "A port of Ubuntu 24.04 (Noble) with a desktop environment",
                    "https://packages.t3gemstone.org/images/ubuntu/noble/t3-gem-o1/desktop.img.xz",
                    "2026-06-08",
                ),
            ],
        );

        let debian = sublist_entry(
            "Debian Images",
            &[
                image_entry(
                    "T3 Gemstone OS (Minimal)",
                    "A port of Debian Bookworm with minimal packages",
                    "https://packages.t3gemstone.org/images/debian/bookworm/t3-gem-o1/min.img.xz",
                    "2026-03-26",
                ),
                image_entry(
                    "T3 Gemstone OS (Desktop)",
                    "A port of Debian Bookworm with a desktop environment",
                    "https://packages.t3gemstone.org/images/debian/bookworm/t3-gem-o1/dsk.img.xz",
                    "2026-03-26",
                ),
            ],
        );

        let pardus = sublist_entry(
            "Pardus Images",
            &[image_entry(
                "T3 Gemstone OS (Minimal)",
                "A port of Pardus Yirmiuc with minimal packages",
                "https://packages.t3gemstone.org/images/pardus-legacy.img.xz",
                "2026-03-26",
            )],
        );

        let ungrouped = image_entry(
            "T3 Gemstone OS (Legacy)",
            "The pre-grouping top-level image",
            "https://packages.t3gemstone.org/images/t3.img.xz",
            "2026-01-01",
        );

        format!(
            r#"{{
              "imager": {{
                "latest_version": "1.0.0",
                "devices": [
                  {{
                    "name": "T3-GEM-O1",
                    "description": "T3 Gemstone board",
                    "tags": ["t3-gem-o1"],
                    "matching_type": "exclusive",
                    "emmc": true,
                    "icon": "https://packages.t3gemstone.org/images/icons/t3.svg"
                  }}
                ]
              }},
              "os_list": [{ungrouped}, {ubuntu}, {debian}, {pardus}]
            }}"#
        )
    }

    fn grouped() -> Config {
        let catalog = grouped_catalog();
        let parsed = parse_catalog(catalog.as_bytes(), ProductScope::T3Only, SOURCE)
            .expect("the grouped catalog shape must parse");
        catalog_to_config(&parsed.catalog)
    }

    fn sublist<'a>(items: &'a [OsListItem], name: &str) -> &'a OsSubList {
        items
            .iter()
            .find_map(|item| match item {
                OsListItem::SubList(list) if list.name == name => Some(list),
                _ => None,
            })
            .unwrap_or_else(|| panic!("expected a sub-list named {name}"))
    }

    fn leaf_names(items: &[OsListItem]) -> Vec<String> {
        items
            .iter()
            .flat_map(|item| match item {
                OsListItem::Image(img) => vec![img.name.clone()],
                OsListItem::SubList(list) => leaf_names(&list.subitems),
                OsListItem::RemoteSubList(_) => Vec::new(),
            })
            .collect()
    }

    #[test]
    fn the_distribution_wrappers_come_back_as_sub_lists() {
        let config = grouped();

        let roots: Vec<&str> = config
            .os_list
            .iter()
            .map(|item| match item {
                OsListItem::Image(img) => img.name.as_str(),
                OsListItem::SubList(list) => list.name.as_str(),
                OsListItem::RemoteSubList(list) => list.name.as_str(),
            })
            .collect();

        assert_eq!(
            roots,
            [
                "T3 Gemstone OS (Legacy)",
                "Ubuntu Images",
                "Debian Images",
                "Pardus Images"
            ]
        );
    }

    #[test]
    fn a_distribution_with_several_releases_gains_a_release_level() {
        let config = grouped();
        let ubuntu = sublist(&config.os_list, "Ubuntu Images");

        let releases: Vec<&str> = ubuntu
            .subitems
            .iter()
            .map(|item| match item {
                OsListItem::SubList(list) => list.name.as_str(),
                other => panic!("expected only release sub-lists, got {other:?}"),
            })
            .collect();

        assert_eq!(releases, ["Ubuntu 24.04 (Noble)", "Ubuntu 22.04 (Jammy)"]);
    }

    #[test]
    fn a_single_release_distribution_stays_flat() {
        let config = grouped();
        let debian = sublist(&config.os_list, "Debian Images");

        assert!(
            debian
                .subitems
                .iter()
                .all(|item| matches!(item, OsListItem::Image(_))),
            "a single-release distribution must not gain a release level"
        );
        assert_eq!(debian.subitems.len(), 2);
    }

    #[test]
    fn no_image_is_lost_when_the_tree_is_rebuilt() {
        let config = grouped();

        assert_eq!(
            leaf_names(&config.os_list).len(),
            9,
            "every in-scope image must survive somewhere in the tree"
        );
    }

    #[test]
    fn an_image_whose_url_has_no_release_segment_is_still_listed() {
        let config = grouped();
        let pardus = sublist(&config.os_list, "Pardus Images");

        assert!(
            matches!(pardus.subitems.as_slice(), [OsListItem::Image(_)]),
            "the image must sit directly under its distribution, not disappear"
        );
    }

    #[test]
    fn variants_are_ordered_with_the_richest_first() {
        let config = grouped();
        let ubuntu = sublist(&config.os_list, "Ubuntu Images");
        let OsListItem::SubList(noble) = &ubuntu.subitems[0] else {
            panic!("expected the newest release to be a sub-list");
        };

        assert_eq!(
            leaf_names(&noble.subitems),
            [
                "T3 Gemstone OS (Desktop)",
                "T3 Gemstone OS (Kiosk)",
                "T3 Gemstone OS (Minimal)"
            ]
        );
    }

    #[test]
    fn the_release_key_comes_from_the_segment_before_the_board_tag() {
        let catalog = grouped_catalog();
        let parsed = parse_catalog(catalog.as_bytes(), ProductScope::T3Only, SOURCE)
            .expect("the grouped catalog shape must parse");

        let noble = parsed
            .catalog
            .images
            .iter()
            .find(|image| image.url.as_str().contains("/noble/"))
            .expect("the fixture publishes a noble image");

        assert_eq!(release_key(noble).as_deref(), Some("noble"));
    }

    #[test]
    fn the_release_label_falls_back_to_the_slug_when_the_description_is_shaped_differently() {
        assert_eq!(
            port_target("A port of Debian Bookworm with extras").as_deref(),
            Some("Debian Bookworm")
        );
        assert_eq!(port_target("Some other description"), None);
        assert_eq!(title_case("yirmiuc-deb"), "Yirmiuc Deb");
    }
}
