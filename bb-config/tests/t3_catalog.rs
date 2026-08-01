//! Contract tests for the strict T3 catalog adapter (the catalog test contract).
//!
//! `main_catalog.json` and `boot_manifest.json` are verbatim captures of the live service taken on
//! 2026-07-31, so these tests double as schema-drift detectors.

use bb_config::t3::{
    DiagnosticSummary, ProductScope, T3_BOARD_TAG, T3CatalogError, T3Diagnostic, WriteMethod,
    parse_catalog,
};

const LIVE_CATALOG: &[u8] = include_bytes!("fixtures/t3/main_catalog.json");
const MISSING_HASH: &[u8] = include_bytes!("fixtures/t3/invalid/missing_hash.json");
const HTTP_DOWNGRADE: &[u8] = include_bytes!("fixtures/t3/invalid/bad_url_downgrade.json");
const ORPHAN_AND_FRIENDS: &[u8] = include_bytes!("fixtures/t3/invalid/orphan_tag.json");

const SOURCE: &str = "https://packages.t3gemstone.org/images/list.json";

fn parse(bytes: &[u8], scope: ProductScope) -> bb_config::t3::T3CatalogParse {
    parse_catalog(bytes, scope, SOURCE).expect("fixture parses")
}

#[test]
fn every_t3_image_in_the_live_catalog_binds_correctly() {
    let parsed = parse(LIVE_CATALOG, ProductScope::T3Only);

    // The live capture holds 34 leaf images, evenly split between the two boards.
    assert_eq!(parsed.catalog.images.len(), 34);
    assert_eq!(parsed.catalog.t3_images().count(), 17);

    for image in parsed.catalog.t3_images() {
        assert!(image.devices.contains(T3_BOARD_TAG));
        assert!(image.integrity.extracted_size > 0);
        assert_eq!(image.url.scheme(), "https");
        // Archive and extracted digests are distinct values in distinct fields.
        assert_ne!(
            image.integrity.archive_sha256,
            image.integrity.extracted_sha256
        );
    }
}

#[test]
fn the_t3_board_gets_a_verified_dfu_profile_and_beagley_stays_sd_only() {
    let parsed = parse(LIVE_CATALOG, ProductScope::T3AndBeagleY);

    let t3 = parsed
        .catalog
        .boards
        .iter()
        .find(|board| board.is_t3())
        .expect("T3 board present");
    let dfu = t3
        .capabilities
        .emmc_dfu
        .as_ref()
        .expect("T3 advertises emmc, so it must carry a verified DFU profile");
    assert_eq!(dfu.vendor_id, 0x0451);
    assert_eq!(dfu.product_id, 0x6165);
    assert_eq!(
        dfu.required_artifacts(),
        ["tiboot3.bin", "tispl.bin", "u-boot.img"]
    );

    let beagley = parsed
        .catalog
        .boards
        .iter()
        .find(|board| !board.is_t3())
        .expect("BeagleY board present");
    assert!(beagley.capabilities.sd);
    assert!(
        !beagley.capabilities.supports_dfu(),
        "BeagleY-AI declares emmc:false and must never offer DFU"
    );
}

#[test]
fn write_methods_come_from_the_board_image_intersection_not_a_single_flasher_field() {
    let parsed = parse(LIVE_CATALOG, ProductScope::T3AndBeagleY);

    let t3 = parsed
        .catalog
        .boards
        .iter()
        .find(|b| b.is_t3())
        .expect("T3 board");
    let beagley = parsed
        .catalog
        .boards
        .iter()
        .find(|b| !b.is_t3())
        .expect("BeagleY board");

    let t3_image = parsed.catalog.t3_images().next().expect("a T3 image");
    let beagley_image = parsed
        .catalog
        .images
        .iter()
        .find(|image| !image.is_t3())
        .expect("a BeagleY image");

    // The same T3 image supports both destinations.
    assert_eq!(
        t3.write_methods_for(t3_image),
        [WriteMethod::Sd, WriteMethod::EmmcDfu]
    );
    // BeagleY is SD-only even though the image itself is otherwise identical in shape.
    assert_eq!(beagley.write_methods_for(beagley_image), [WriteMethod::Sd]);
    // A board never offers a write method for an image that does not name it.
    assert!(t3.write_methods_for(beagley_image).is_empty());
}

#[test]
fn t3_only_scope_marks_beagley_out_of_scope_without_dropping_it() {
    let parsed = parse(LIVE_CATALOG, ProductScope::T3Only);

    // Both real boards survive parsing; only visibility changes.
    assert_eq!(parsed.catalog.boards.len(), 2);
    assert_eq!(parsed.catalog.boards_in_scope().count(), 1);
    assert!(parsed.catalog.boards_in_scope().all(|board| board.is_t3()));

    assert!(parsed.diagnostics.iter().any(
        |d| matches!(d, T3Diagnostic::OutOfProductScope { board, .. } if board == "BeagleY-AI")
    ));
}

#[test]
fn the_tagless_pseudo_device_is_rejected_with_a_path_not_silently_skipped() {
    let parsed = parse(LIVE_CATALOG, ProductScope::T3Only);

    assert_eq!(parsed.rejected_boards, 1);
    let rejection = parsed
        .diagnostics
        .iter()
        .find(|d| matches!(d, T3Diagnostic::BoardWithoutTags { .. }))
        .expect("the tagless `No filtering` entry is reported");
    assert_eq!(rejection.path(), "imager.devices[0]");
}

#[test]
fn missing_required_field_is_reported_with_its_json_path() {
    let parsed = parse(MISSING_HASH, ProductScope::T3Only);

    assert_eq!(parsed.rejected_images, 1);
    let diagnostic = parsed
        .diagnostics
        .iter()
        .find(
            |d| matches!(d, T3Diagnostic::MissingField { field, .. } if *field == "extract_sha256"),
        )
        .expect("missing extract_sha256 is reported");
    assert_eq!(diagnostic.path(), "os_list[0]");

    // The healthy neighbour still comes through: one bad entry does not poison the catalog.
    assert_eq!(parsed.catalog.images.len(), 1);
}

#[test]
fn https_downgrade_is_rejected() {
    let parsed = parse(HTTP_DOWNGRADE, ProductScope::T3Only);

    assert_eq!(parsed.rejected_images, 1);
    assert!(parsed.diagnostics.iter().any(|d| {
        matches!(
            d,
            T3Diagnostic::InsecureUrl { scheme, field, .. } if scheme == "http" && *field == "url"
        )
    }));
    assert_eq!(parsed.catalog.images.len(), 1);
}

#[test]
fn orphan_tag_zero_size_and_short_hash_are_each_rejected_with_a_reason() {
    let parsed = parse(ORPHAN_AND_FRIENDS, ProductScope::T3Only);

    assert_eq!(parsed.rejected_images, 3);
    assert_eq!(parsed.catalog.images.len(), 1);

    assert!(parsed.diagnostics.iter().any(
        |d| matches!(d, T3Diagnostic::OrphanDeviceTag { tag, .. } if tag == "t3-gem-nonexistent")
    ));
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|d| matches!(d, T3Diagnostic::ZeroExtractSize { .. }))
    );
    assert!(parsed.diagnostics.iter().any(
        |d| matches!(d, T3Diagnostic::InvalidSha256 { field, .. } if *field == "extract_sha256")
    ));

    let summary = DiagnosticSummary::of(&parsed.diagnostics);
    assert!(summary.has_rejections());
}

#[test]
fn an_empty_catalog_is_an_error_not_a_success() {
    let err = parse_catalog(b"{}", ProductScope::T3Only, SOURCE)
        .expect_err("an empty document must not parse as a usable catalog");
    assert!(matches!(err, T3CatalogError::NoBoards { .. }));
}

#[test]
fn a_catalog_with_boards_but_no_usable_image_is_an_error() {
    let json = br#"{
        "imager": {"devices": [{"name": "T3-GEM-O1", "tags": ["t3-gem-o1"], "emmc": true}]},
        "os_list": []
    }"#;
    let err = parse_catalog(json, ProductScope::T3Only, SOURCE)
        .expect_err("no usable image must not parse as success");
    assert!(matches!(err, T3CatalogError::NoUsableImages { .. }));
}

#[test]
fn malformed_json_is_reported_as_a_json_error() {
    let err = parse_catalog(b"{not json", ProductScope::T3Only, SOURCE)
        .expect_err("malformed JSON must fail");
    assert!(matches!(err, T3CatalogError::Json(_)));
}

#[test]
fn unknown_future_fields_are_ignored_for_forward_compatibility() {
    // `random` already exists on live sub-list wrappers; a future scalar field must not break us.
    let json = br#"{
        "imager": {
            "devices": [{"name": "T3-GEM-O1", "tags": ["t3-gem-o1"], "emmc": true}],
            "some_future_key": 42
        },
        "os_list": [{
            "name": "T3 Gemstone OS (Minimal)",
            "description": "d",
            "devices": ["t3-gem-o1"],
            "url": "https://packages.t3gemstone.org/images/x.img.xz",
            "image_download_sha256": "13e237518eee97dead84f2d8009a6d875516e1ab2eb800c90fe44f5ff49774c7",
            "image_download_size": 1,
            "extract_sha256": "9c991802d2ceff5a80cfd3e822f9cd2f9730cee59759714ba7530c81a824c92d",
            "extract_size": 2,
            "release_date": "2026-03-26",
            "init_format": "systemd",
            "brand_new_field": {"nested": true}
        }]
    }"#;

    let parsed = parse(json, ProductScope::T3Only);
    assert_eq!(parsed.catalog.images.len(), 1);
    assert_eq!(parsed.rejected_images, 0);
}

#[test]
fn sublist_images_keep_their_group() {
    let parsed = parse(LIVE_CATALOG, ProductScope::T3Only);

    let grouped = parsed
        .catalog
        .images
        .iter()
        .find(|image| image.group.as_deref() == Some("Pardus Images"))
        .expect("the live catalog publishes a Pardus sub-list");
    assert!(grouped.integrity.extracted_size > 0);
}

#[test]
fn desktop_images_are_flagged_for_vnc_and_minimal_images_are_not() {
    let parsed = parse(LIVE_CATALOG, ProductScope::T3Only);

    let desktop = parsed
        .catalog
        .t3_images()
        .find(|image| image.name.contains("Desktop"))
        .expect("a desktop image");
    let minimal = parsed
        .catalog
        .t3_images()
        .find(|image| image.name.contains("Minimal"))
        .expect("a minimal image");

    assert!(
        desktop
            .customization
            .as_ref()
            .expect("systemd images are customizable")
            .desktop_variant
    );
    assert!(
        !minimal
            .customization
            .as_ref()
            .expect("systemd images are customizable")
            .desktop_variant
    );
}

#[test]
fn an_unknown_init_format_disables_customization_but_keeps_the_image() {
    let json = br#"{
        "imager": {"devices": [{"name": "T3-GEM-O1", "tags": ["t3-gem-o1"], "emmc": true}]},
        "os_list": [{
            "name": "Future image",
            "devices": ["t3-gem-o1"],
            "url": "https://packages.t3gemstone.org/images/x.img.xz",
            "image_download_sha256": "13e237518eee97dead84f2d8009a6d875516e1ab2eb800c90fe44f5ff49774c7",
            "extract_sha256": "9c991802d2ceff5a80cfd3e822f9cd2f9730cee59759714ba7530c81a824c92d",
            "extract_size": 2,
            "release_date": "2026-03-26",
            "init_format": "cloudinit"
        }]
    }"#;

    let parsed = parse(json, ProductScope::T3Only);
    assert_eq!(parsed.catalog.images.len(), 1, "image stays flashable");
    assert!(parsed.catalog.images[0].customization.is_none());
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|d| matches!(d, T3Diagnostic::UnsupportedInitFormat { .. }))
    );
}

#[test]
fn a_non_t3_board_claiming_emmc_does_not_get_a_dfu_profile() {
    let json = br#"{
        "imager": {"devices": [
            {"name": "T3-GEM-O1", "tags": ["t3-gem-o1"], "emmc": true},
            {"name": "Mystery board", "tags": ["mystery"], "emmc": true}
        ]},
        "os_list": [{
            "name": "T3 image",
            "devices": ["t3-gem-o1"],
            "url": "https://packages.t3gemstone.org/images/x.img.xz",
            "image_download_sha256": "13e237518eee97dead84f2d8009a6d875516e1ab2eb800c90fe44f5ff49774c7",
            "extract_sha256": "9c991802d2ceff5a80cfd3e822f9cd2f9730cee59759714ba7530c81a824c92d",
            "extract_size": 2,
            "release_date": "2026-03-26",
            "init_format": "systemd"
        }]
    }"#;

    let parsed = parse(json, ProductScope::T3Only);
    let mystery = parsed
        .catalog
        .boards
        .iter()
        .find(|board| board.name == "Mystery board")
        .expect("board is retained");

    assert!(mystery.capabilities.sd, "SD support is unaffected");
    assert!(
        !mystery.capabilities.supports_dfu(),
        "no verified boot manifest exists for a non-T3 board"
    );
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|d| matches!(d, T3Diagnostic::EmmcWithoutVerifiedDfuProfile { .. }))
    );
}

#[test]
fn provenance_records_where_the_catalog_came_from() {
    let parsed = parse(LIVE_CATALOG, ProductScope::T3Only);
    assert_eq!(parsed.catalog.provenance.source_url, SOURCE);
    assert_eq!(
        parsed.catalog.provenance.latest_version.as_deref(),
        Some("1.1.1")
    );
}

/// End-to-end proof against the captured live catalog: the two product boards, and only those,
/// reach the model the front-end screens actually render.
///
/// This is the test that would have caught the fork's headline defect. The adapter and the
/// canonical model were correct all along; nothing translated them into the front-end model, so
/// the board list was empty no matter what the catalog said.
#[test]
fn the_live_catalog_reaches_the_front_end_model_with_both_product_boards() {
    let parsed = parse(LIVE_CATALOG, ProductScope::T3AndBeagleY);
    let config = bb_config::t3::catalog_to_config(&parsed.catalog);

    let mut boards: Vec<&str> = config
        .imager
        .devices
        .iter()
        .map(|d| d.name.as_str())
        .collect();
    boards.sort_unstable();

    assert_eq!(
        boards,
        ["BeagleY-AI", "T3-GEM-O1"],
        "the product surface is exactly these two boards"
    );

    // The catalog also publishes a tagless "No filtering" pseudo-device. It must not become a
    // selectable board.
    assert!(
        config.imager.devices.iter().all(|d| !d.tags.is_empty()),
        "a tagless pseudo-device must never reach the board list"
    );

    assert!(
        !config.os_list.is_empty(),
        "an empty image list is never a success"
    );

    // Every emitted image belongs to one of the two boards.
    let board_tags: std::collections::HashSet<&str> = config
        .imager
        .devices
        .iter()
        .flat_map(|d| d.tags.iter().map(String::as_str))
        .collect();
    for item in &config.os_list {
        let bb_config::config::OsListItem::Image(img) = item else {
            panic!("the bridge only emits plain images");
        };
        assert!(
            img.devices.iter().any(|d| board_tags.contains(d.as_str())),
            "image \"{}\" targets no board in the product surface",
            img.name
        );
        // The decoder and device read-back layers verify against these; an image that arrives without them flashes unverified.
        assert!(
            img.extract_sha256.is_some(),
            "{} lost its extracted digest",
            img.name
        );
        assert!(img.extract_size > 0, "{} lost its extracted size", img.name);
    }
}

/// The T3 board must carry at least one image of its own, otherwise selecting it is a dead end.
#[test]
fn the_t3_board_has_images_in_the_front_end_model() {
    let parsed = parse(LIVE_CATALOG, ProductScope::T3AndBeagleY);
    let config = bb_config::t3::catalog_to_config(&parsed.catalog);

    let t3_images: Vec<&str> = config
        .os_list
        .iter()
        .filter_map(|item| match item {
            bb_config::config::OsListItem::Image(img) if img.devices.contains(T3_BOARD_TAG) => {
                Some(img.name.as_str())
            }
            _ => None,
        })
        .collect();

    assert!(
        !t3_images.is_empty(),
        "T3-GEM-O1 must have at least one selectable image"
    );
}
