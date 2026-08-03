#![cfg(feature = "store")]

//! Integration tests for the public `bb_config` parsing/serialization API.
//!
//! The checked-in `config.json` fixture has an empty `os_list`, so the crate's
//! inline `basic` test never exercises `OsImage`/`OsListItem` deserialization,
//! the `serde`/`rusqlite` round-trips, or the untagged-enum disambiguation.
//! These tests cover those paths using self-contained JSON literals.

use bb_config::config::{Config, Flasher, InitFormat, OsListItem};

/// A minimal but complete `OsImage` JSON object (all required fields present).
const OS_IMAGE_JSON: &str = r#"{
    "name": "Test Image",
    "description": "an image",
    "icon": "https://example.com/icon.png",
    "url": "https://example.com/image.img.xz",
    "image_download_size": 4096,
    "image_download_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
    "extract_size": 8192,
    "release_date": "2024-01-02",
    "devices": ["board-a", "board-b"],
    "init_format": "sysconf"
}"#;

/// A `OsSubList` JSON object: has `subitems`, no `subitems_url`.
const OS_SUBLIST_JSON: &str = r#"{
    "name": "Testing",
    "description": "nested list",
    "icon": "https://example.com/sub.png",
    "subitems": []
}"#;

/// A `OsRemoteSubList` JSON object: has `devices` + `subitems_url`, no `subitems`.
const OS_REMOTE_SUBLIST_JSON: &str = r#"{
    "name": "Remote",
    "description": "remote list",
    "icon": "https://example.com/remote.png",
    "devices": ["board-a"],
    "subitems_url": "https://example.com/remote.json"
}"#;

fn config_with_os_list(items_json: &str) -> Config {
    let doc = format!(r#"{{ "imager": {{}}, "os_list": {items_json} }}"#);
    serde_json::from_str(&doc).expect("config should deserialize")
}

#[test]
fn full_config_round_trip() {
    // Deserialize a rich document, then serialize -> deserialize again and
    // assert equality. This exercises both `Serialize` and `Deserialize` for
    // `Config` and every nested type without needing manual construction.
    let doc = format!(
        r#"{{
            "imager": {{
                "remote_configs": ["https://example.com/extra.json"],
                "devices": [{{
                    "name": "Board A",
                    "tags": ["a", "b"],
                    "icon": "https://example.com/board.png",
                    "description": "a board",
                    "flasher": "SdCard",
                    "documentation": "https://docs.example.com/",
                    "instructions": "hold the button",
                    "specification": {{ "ram": "4GB", "cpu": "arm" }},
                    "oshw": "OSHW-1"
                }}]
            }},
            "os_list": [{OS_IMAGE_JSON}, {OS_SUBLIST_JSON}, {OS_REMOTE_SUBLIST_JSON}]
        }}"#
    );

    let first: Config = serde_json::from_str(&doc).expect("first parse");
    let serialized = serde_json::to_string(&first).expect("serialize");
    let second: Config = serde_json::from_str(&serialized).expect("re-parse");

    assert_eq!(
        first, second,
        "config should survive a serialize round-trip"
    );

    // Spot-check that the interesting fields actually populated.
    assert_eq!(first.os_list.len(), 3);
    assert_eq!(first.imager.devices.len(), 1);
    let device = &first.imager.devices[0];
    assert_eq!(device.flasher, Flasher::SdCard);
    // `specification` is (de)serialized as a map via `serde_with::Map`, preserving insertion order.
    assert_eq!(
        device.specification,
        vec![
            ("ram".to_string(), "4GB".to_string()),
            ("cpu".to_string(), "arm".to_string()),
        ]
    );
}

#[test]
fn os_list_item_untagged_disambiguation() {
    // The untagged enum must pick the right variant purely from field shape.
    match &config_with_os_list(&format!("[{OS_IMAGE_JSON}]")).os_list[0] {
        OsListItem::Image(img) => {
            assert_eq!(img.name, "Test Image");
            assert_eq!(img.extract_size, 8192);
            assert_eq!(img.init_format, InitFormat::Sysconf);
        }
        other => panic!("expected Image, got {other:?}"),
    }

    match &config_with_os_list(&format!("[{OS_SUBLIST_JSON}]")).os_list[0] {
        OsListItem::SubList(sub) => assert_eq!(sub.name, "Testing"),
        other => panic!("expected SubList, got {other:?}"),
    }

    match &config_with_os_list(&format!("[{OS_REMOTE_SUBLIST_JSON}]")).os_list[0] {
        OsListItem::RemoteSubList(remote) => assert_eq!(remote.name, "Remote"),
        other => panic!("expected RemoteSubList, got {other:?}"),
    }
}

#[test]
fn vec_skip_error_drops_malformed_items() {
    // `os_list` is wrapped in VecSkipError: a malformed entry is silently
    // dropped rather than failing the whole parse.
    let config = config_with_os_list(&format!(r#"[{OS_IMAGE_JSON}, {{"garbage": true}}]"#));
    assert_eq!(
        config.os_list.len(),
        1,
        "the malformed entry should be skipped, leaving only the valid image"
    );
    assert!(matches!(config.os_list[0], OsListItem::Image(_)));
}

#[test]
fn init_format_serde_strings() {
    // Documents the *live* serde representation (distinct from the unused
    // Display impl, which renders Sysconf as "sysconfig").
    let cases = [
        (InitFormat::None, "\"none\""),
        (InitFormat::Sysconf, "\"sysconf\""),
        (InitFormat::Armbian, "\"armbian\""),
        (InitFormat::CloudInit, "\"cloudinit\""),
    ];
    for (variant, expected) in cases {
        assert_eq!(serde_json::to_string(&variant).unwrap(), expected);
        assert_eq!(
            serde_json::from_str::<InitFormat>(expected).unwrap(),
            variant
        );
    }
}

#[test]
fn flasher_serde_strings() {
    // Flasher has no rename attribute: it serializes as the variant identifier.
    let cases = [(Flasher::SdCard, "\"SdCard\"")];
    for (variant, expected) in cases {
        assert_eq!(serde_json::to_string(&variant).unwrap(), expected);
        assert_eq!(serde_json::from_str::<Flasher>(expected).unwrap(), variant);
    }
}

#[test]
fn flasher_sqlite_round_trip() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute("CREATE TABLE t (id INTEGER, f)", []).unwrap();

    let variants = [Flasher::SdCard];
    for (i, variant) in variants.iter().enumerate() {
        conn.execute(
            "INSERT INTO t (id, f) VALUES (?1, ?2)",
            rusqlite::params![i as i64, variant],
        )
        .unwrap();
    }

    let mut stmt = conn.prepare("SELECT f FROM t ORDER BY id").unwrap();
    let got: Vec<Flasher> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(got, variants);
}

#[test]
fn init_format_sqlite_round_trip() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute("CREATE TABLE t (id INTEGER, f)", []).unwrap();

    let variants = [
        InitFormat::None,
        InitFormat::Sysconf,
        InitFormat::Armbian,
        InitFormat::CloudInit,
    ];
    for (i, variant) in variants.iter().enumerate() {
        conn.execute(
            "INSERT INTO t (id, f) VALUES (?1, ?2)",
            rusqlite::params![i as i64, variant],
        )
        .unwrap();
    }

    let mut stmt = conn.prepare("SELECT f FROM t ORDER BY id").unwrap();
    let got: Vec<InitFormat> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(got, variants);
}

#[test]
fn sqlite_invalid_discriminant_errors() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute("CREATE TABLE t (f)", []).unwrap();
    // 99 maps to no valid variant for either enum.
    conn.execute("INSERT INTO t (f) VALUES (99)", []).unwrap();

    let flasher: rusqlite::Result<Flasher> =
        conn.query_row("SELECT f FROM t", [], |row| row.get(0));
    assert!(
        flasher.is_err(),
        "an out-of-range discriminant must fail FromSql for Flasher"
    );

    let init: rusqlite::Result<InitFormat> =
        conn.query_row("SELECT f FROM t", [], |row| row.get(0));
    assert!(
        init.is_err(),
        "an out-of-range discriminant must fail FromSql for InitFormat"
    );
}

// ---------------------------------------------------------------------------
// Extracted-side digest (`instruction.md` §8.1)
// ---------------------------------------------------------------------------

/// The T3 catalog publishes `extract_sha256`; the legacy schema never did. Both have to survive
/// the same parser, and "absent" must stay distinguishable from "present", because an all-zero
/// default would turn a missing gate into a gate that can never pass.
#[test]
fn the_extracted_digest_is_carried_when_the_catalog_publishes_it() {
    let with_digest = config_with_os_list(
        r#"[{
            "name": "T3 Gemstone OS (Minimal)",
            "description": "A port of Ubuntu 22.04",
            "icon": "https://packages.t3gemstone.org/images/icons/ubuntu.svg",
            "url": "https://packages.t3gemstone.org/images/os.img.xz",
            "image_download_size": 138737372,
            "image_download_sha256": "13e237518eee97dead84f2d8009a6d875516e1ab2eb800c90fe44f5ff49774c7",
            "extract_sha256": "9c991802d2ceff5a80cfd3e822f9cd2f9730cee59759714ba7530c81a824c92d",
            "extract_size": 1024000000,
            "release_date": "2026-03-26",
            "devices": ["t3-gem-o1"]
        }]"#,
    );

    let OsListItem::Image(image) = &with_digest.os_list[0] else {
        panic!("expected an image entry");
    };

    let extract_sha256 = image.extract_sha256.expect("the digest must be carried");
    assert_eq!(
        const_hex::encode(extract_sha256),
        "9c991802d2ceff5a80cfd3e822f9cd2f9730cee59759714ba7530c81a824c92d"
    );
    // The two digests are distinct values and must never be conflated.
    assert_ne!(extract_sha256, image.image_download_sha256);
}

#[test]
fn a_catalog_entry_without_an_extracted_digest_parses_as_absent() {
    let without_digest = config_with_os_list(
        r#"[{
            "name": "Legacy image",
            "description": "No extracted digest published",
            "icon": "https://example.com/icon.png",
            "url": "https://example.com/os.img.xz",
            "image_download_sha256": "13e237518eee97dead84f2d8009a6d875516e1ab2eb800c90fe44f5ff49774c7",
            "extract_size": 1024,
            "release_date": "2026-03-26",
            "devices": ["beagley-ai"]
        }]"#,
    );

    let OsListItem::Image(image) = &without_digest.os_list[0] else {
        panic!("expected an image entry");
    };

    assert_eq!(image.extract_sha256, None);
}

/// An unusable digest makes the whole entry unparsable — and the legacy list drops unparsable
/// entries silently, which is precisely the behaviour `instruction.md` §6.1 forbids for the T3
/// adapter. Documented here so the difference between the two parsers stays visible: an image
/// whose digest cannot be read simply disappears from this one.
#[test]
fn an_unusable_extracted_digest_makes_the_legacy_parser_drop_the_entry() {
    let broken = config_with_os_list(
        r#"[{
            "name": "Broken image",
            "description": "Digest is not hex",
            "icon": "https://example.com/icon.png",
            "url": "https://example.com/os.img.xz",
            "image_download_sha256": "13e237518eee97dead84f2d8009a6d875516e1ab2eb800c90fe44f5ff49774c7",
            "extract_sha256": "definitely not a sha256",
            "extract_size": 1024,
            "release_date": "2026-03-26",
            "devices": ["t3-gem-o1"]
        }]"#,
    );

    assert!(
        broken.os_list.is_empty(),
        "the legacy list skips entries it cannot parse"
    );
}
