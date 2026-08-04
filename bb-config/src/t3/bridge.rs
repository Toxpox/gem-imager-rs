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
//! # How capability crosses the bridge
//!
//! [`crate::config::Flasher`] can only express `SdCard`, so the DFU capability travels as the
//! separate [`crate::config::Device::emmc_dfu`] flag rather than as another flasher variant: the
//! same T3 image is writable over SD *and* over DFU, which makes the write method a property of
//! the board/image pair, not of the image (`instruction.md` §6.3). The full [`DfuProfile`] stays in
//! the canonical model — the front-end only needs to know that the destination may be offered.
//!
//! # How the list regains its shape
//!
//! The catalog publishes its images inside `subitems` wrappers — "Debian Images", "Ubuntu Images",
//! "Pardus Images". [`crate::t3::validate`] flattens those on purpose: an image is validated the
//! same way wherever it sits, and the wrapper survives only as [`Image::group`]. Rebuilding the
//! tree is therefore this module's job, and it happens here rather than in the GUI because the
//! front-end already renders arbitrarily deep [`OsListItem::SubList`] trees — it just never
//! received one.
//!
//! Two levels are rebuilt:
//!
//! 1. **Distribution** — straight from [`Image::group`].
//! 2. **Release** — derived from the URL path, because the catalog publishes no release field.
//!    Every T3 image name is one of three strings (`(Minimal)`, `(Kiosk)`, `(Desktop)`), so
//!    without this level the same name appears six times with no way to tell Jammy from Noble.
//!
//! Derivation is confined to [`release_key`] and never drops an image: one whose URL yields no
//! release sits directly under its distribution instead of vanishing. A vanishing image is the
//! exact failure this adapter exists to stop.
//!
//! [`DfuProfile`]: crate::t3::canonical::DfuProfile

use std::collections::HashSet;

use url::Url;

use crate::config::{Config, Device, Flasher, Imager, InitFormat, OsImage, OsListItem, OsSubList};
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

    let in_scope: Vec<&Image> = catalog
        .images
        .iter()
        .filter(|image| boards.iter().any(|board| board.accepts(image)))
        .collect();

    let os_list = group_os_list(&in_scope, &boards);

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
        // `Flasher` still only names the SD path; the DFU capability travels beside it.
        flasher: Flasher::SdCard,
        // Only a board with a *verified* DFU profile reports the capability. `emmc: true` in the
        // catalog is not enough on its own — `board_capabilities` refuses to attach a profile to a
        // board this build has no verified contract for.
        emmc_dfu: board.capabilities.supports_dfu(),
        documentation: None,
        instructions: None,
        specification: Vec::new(),
        oshw: None,
    }
}

/// Which customization screen the front-end offers for an image.
///
/// The T3 catalog's `init_format: "systemd"` names the consumer, not a file format: it is the
/// GemInit `config.ini` writer. That is deliberately **not** mapped onto
/// [`crate::config::InitFormat::Sysconf`], which is BeagleBoard's `sysconf.txt` and has a different
/// key set — writing one where the other is expected produces a board that ignores the file.
///
/// An image with no recognised consumer reports [`crate::config::InitFormat::None`], which offers
/// no customization at all. That is the safe direction: nothing is written rather than something
/// the board cannot read.
fn init_format(image: &Image) -> InitFormat {
    match &image.customization {
        Some(profile) if profile.desktop_variant => InitFormat::GemInitDesktop,
        Some(_) => InitFormat::GemInit,
        None => InitFormat::None,
    }
}

/// Translate one image, resolving the icon the front-end requires.
///
/// [`OsImage::icon`] is mandatory while the canonical model allows it to be absent. Rather than
/// invent a URL, the icon falls back to that of a board which accepts the image. An image with no
/// icon anywhere is skipped — but loudly, never silently, because a vanishing image is the exact
/// failure this module exists to stop.
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

/// The icon the front-end model requires, falling back to a board that accepts the image.
///
/// Rather than invent a URL, the icon falls back to that of a board which accepts the image.
fn resolve_icon(image: &Image, boards: &[&Board]) -> Option<Url> {
    image.icon.clone().or_else(|| {
        boards
            .iter()
            .find(|board| board.accepts(image))
            .and_then(|board| board.icon.clone())
    })
}

/// Rebuild the distribution/release tree the catalog publishes and [`crate::t3::validate`] flattens.
///
/// Catalog order is preserved: an ungrouped image keeps its slot, and a distribution occupies the
/// slot of its first member. That way the list the user sees still tracks the order the catalog
/// author chose, rather than an alphabetical order this module invented.
fn group_os_list(images: &[&Image], boards: &[&Board]) -> Vec<OsListItem> {
    /// A root slot, held in catalog order before the groups are turned into sub-lists.
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

/// One distribution level, with a release level inside it when there is more than one release.
///
/// A single-release distribution collapses to a flat list of its images. Adding a level that only
/// ever holds one child costs the user a click and shows them nothing they did not already know.
fn distribution_sublist(name: &str, members: &[&Image], boards: &[&Board]) -> Option<OsListItem> {
    let Some(icon) = members.iter().find_map(|image| resolve_icon(image, boards)) else {
        tracing::warn!(
            "Skipping T3 distribution \"{name}\": no image under it publishes an icon, and the \
             front-end model requires one"
        );
        return None;
    };

    // Releases in first-appearance order, plus the images whose URL yielded no release at all.
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

    // Newest release first; the user almost always wants the current one.
    releases.sort_by(|a, b| newest_release_date(&b.1).cmp(&newest_release_date(&a.1)));

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

    // Never nested behind a release they have no key for.
    subitems.extend(sorted_images(&undated, boards));

    if subitems.is_empty() {
        tracing::warn!("Skipping T3 distribution \"{name}\": every image under it was dropped");
        return None;
    }

    Some(OsListItem::SubList(OsSubList {
        name: name.to_owned(),
        // `OsSubList::description` is metadata: the front-end list pane renders only name and icon.
        // Listing the releases keeps it honest and derived rather than invented.
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

/// One release level, holding the variants (Desktop/Kiosk/Minimal) published for it.
fn release_sublist(key: &str, images: &[&Image], boards: &[&Board]) -> Option<OsListItem> {
    let icon = images.iter().find_map(|image| resolve_icon(image, boards))?;
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

/// Images in the order a user reads them: richest variant first, then by name for anything else.
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

/// Which release an image belongs to, derived from its URL.
///
/// The catalog publishes no release field, so the URL is the only stable carrier:
/// `.../images/{distro}/{release}/{board-tag}/{file}`. The board tag is located by matching against
/// the image's own `devices`, so the segment *before* it is the release — no absolute position is
/// assumed, and a URL that gains or loses a leading segment still resolves.
///
/// Returns `None` when the shape does not hold. Callers must list the image anyway.
fn release_key(image: &Image) -> Option<String> {
    let segments: Vec<&str> = image
        .url
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .collect();

    let board_position = segments
        .iter()
        .position(|segment| image.devices.contains(*segment))?;

    // A board tag in the first segment leaves no room for a release before it.
    let release = segments.get(board_position.checked_sub(1)?)?;

    (!release.is_empty()).then(|| release.to_lowercase())
}

/// The human-facing name of a release.
///
/// The catalog descriptions read `"A port of Ubuntu 22.04 (Jammy) with minimal packages"`, so the
/// text between the two markers is the release as its publisher names it. That is preferred over
/// anything derived from the URL slug, which is an implementation detail of the file layout.
fn release_label(key: &str, images: &[&Image]) -> String {
    images
        .iter()
        .find_map(|image| port_target(&image.description))
        .unwrap_or_else(|| title_case(key))
}

/// Extract `X` from `"A port of X with ..."`, or `None` when the description is shaped differently.
fn port_target(description: &str) -> Option<String> {
    const PREFIX: &str = "A port of ";
    const SUFFIX: &str = " with ";

    let rest = description.strip_prefix(PREFIX)?;
    let end = rest.find(SUFFIX)?;
    let target = rest[..end].trim();

    (!target.is_empty()).then(|| target.to_owned())
}

/// Fallback display name for a release slug: `jammy-deb` becomes `Jammy Deb`.
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

/// Variant ordering. Unrecognised names sort last rather than being reordered arbitrarily.
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

    /// `systemd` is the GemInit consumer, not BeagleBoard's `sysconf.txt`. Mapping it onto the
    /// BeagleBoard formats would make the GUI write a file the board never reads.
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

    /// VNC is only offered on desktop images (`instruction.md` §10.1), and desktop-ness comes from
    /// the canonical model rather than being re-derived here.
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

    /// The destination screen decides whether to offer DFU from this flag, so it has to survive the
    /// bridge — and only for the board that has a verified profile.
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
        // `emmc: false` in the catalog, so DFU must never be offered for it.
        assert!(!beagley.emmc_dfu);
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

    // ---------------------------------------------------------------------------------------
    // Distribution / release grouping
    // ---------------------------------------------------------------------------------------

    const HASH_ARCHIVE: &str =
        "668a83c94264c17e9e549284b50ec1f9ec1c0a1d171ede3a92797a458eabc198";
    const HASH_EXTRACTED: &str =
        "33afbc809f8c39c4a7472c49e26f7c5ac507c5b1d97df05c42ec83e97e1f6e51";

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

    /// A catalog in the live document's *grouped* shape: `subitems` wrappers per distribution, with
    /// the release carried only by the URL path and the release name only by the description.
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

        // Deliberately not in the `{distro}/{release}/{board}/` shape: the release cannot be
        // derived, and the image must still be listed.
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

    /// Count the images at the leaves, however deep the tree goes.
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

    /// The headline behaviour: the wrappers the catalog publishes come back as a browsable level
    /// instead of being flattened into one long list.
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

        // Catalog order is preserved, and the ungrouped image keeps its own slot.
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

    /// Without this level the user sees "T3 Gemstone OS (Desktop)" twice under Ubuntu with no way
    /// to tell Jammy from Noble — the names are identical and only the URL differs.
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

        // Named as the publisher names them, newest first.
        assert_eq!(releases, ["Ubuntu 24.04 (Noble)", "Ubuntu 22.04 (Jammy)"]);
    }

    /// A level that can only ever hold one child costs a click and teaches the user nothing.
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

    /// The silent-drop guard. Rebuilding the tree must move images, never lose them.
    #[test]
    fn no_image_is_lost_when_the_tree_is_rebuilt() {
        let config = grouped();

        assert_eq!(
            leaf_names(&config.os_list).len(),
            9,
            "every in-scope image must survive somewhere in the tree"
        );
    }

    /// Derivation is best-effort by design: an unparseable URL demotes the image one level, it does
    /// not remove it.
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
        assert_eq!(port_target("A port of Debian Bookworm with extras").as_deref(), Some("Debian Bookworm"));
        assert_eq!(port_target("Some other description"), None);
        assert_eq!(title_case("yirmiuc-deb"), "Yirmiuc Deb");
    }
}
