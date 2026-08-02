#![cfg(feature = "store")]

//! Persistence and migration tests for the T3 catalog store (`instruction.md` §6.4, §6.5).

use bb_config::t3::store::{CURRENT_SCHEMA_VERSION, HttpValidators, StoreError, T3CatalogStore};
use bb_config::t3::{ProductScope, ValidatedT3Catalog, parse_catalog};
use chrono::NaiveDate;

const LIVE_CATALOG: &[u8] = include_bytes!("fixtures/t3/main_catalog.json");
const SOURCE: &str = "https://packages.t3gemstone.org/images/list.json";

fn fetched_at() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 7, 31).expect("valid date")
}

fn live_catalog(scope: ProductScope) -> ValidatedT3Catalog {
    parse_catalog(LIVE_CATALOG, scope, SOURCE)
        .expect("fixture parses")
        .catalog
}

#[test]
fn a_fresh_store_is_migrated_to_the_current_schema_version() {
    let store = T3CatalogStore::open_in_memory().expect("store opens");
    assert_eq!(
        store.schema_version().expect("version readable"),
        CURRENT_SCHEMA_VERSION
    );
}

#[test]
fn an_empty_store_loads_nothing_rather_than_an_empty_catalog() {
    let store = T3CatalogStore::open_in_memory().expect("store opens");
    assert!(store.load().expect("load succeeds").is_none());
    assert!(store.stored_scope().expect("scope readable").is_none());
}

#[test]
fn the_whole_catalog_round_trips_including_board_capabilities() {
    let original = live_catalog(ProductScope::T3AndBeagleY);
    let mut store = T3CatalogStore::open_in_memory().expect("store opens");
    store
        .save(&original, ProductScope::T3AndBeagleY, fetched_at())
        .expect("save succeeds");

    let loaded = store
        .load()
        .expect("load succeeds")
        .expect("a catalog was saved");

    assert_eq!(loaded.boards.len(), original.boards.len());
    assert_eq!(loaded.images.len(), original.images.len());
    assert_eq!(loaded.provenance, original.provenance);
    // Boards and images are stored and reloaded in document order.
    assert_eq!(loaded.boards, original.boards);
    assert_eq!(loaded.images, original.images);
}

#[test]
fn the_dfu_profile_survives_a_round_trip_with_its_stage_order() {
    let original = live_catalog(ProductScope::T3Only);
    let mut store = T3CatalogStore::open_in_memory().expect("store opens");
    store
        .save(&original, ProductScope::T3Only, fetched_at())
        .expect("save succeeds");

    let loaded = store.load().expect("load succeeds").expect("catalog saved");
    let t3 = loaded
        .boards
        .iter()
        .find(|board| board.is_t3())
        .expect("T3 board");
    let dfu = t3
        .capabilities
        .emmc_dfu
        .as_ref()
        .expect("DFU profile survives persistence");

    assert_eq!(dfu.vendor_id, 0x0451);
    assert_eq!(dfu.product_id, 0x6165);
    assert_eq!(
        dfu.required_artifacts(),
        ["tiboot3.bin", "tispl.bin", "u-boot.img"],
        "stage order must be preserved, not re-sorted by name"
    );
    assert_eq!(dfu.stages[0].alt_setting, "bootloader");
    assert_eq!(dfu.raw_emmc_alt_setting, "rawemmc");

    if let Some(beagley) = loaded.boards.iter().find(|board| !board.is_t3()) {
        assert!(!beagley.capabilities.supports_dfu());
    }
}

#[test]
fn all_four_integrity_gates_are_stored_separately() {
    let original = live_catalog(ProductScope::T3Only);
    let mut store = T3CatalogStore::open_in_memory().expect("store opens");
    store
        .save(&original, ProductScope::T3Only, fetched_at())
        .expect("save succeeds");

    let loaded = store.load().expect("load succeeds").expect("catalog saved");

    for (before, after) in original.images.iter().zip(loaded.images.iter()) {
        assert_eq!(
            before.integrity.archive_sha256,
            after.integrity.archive_sha256
        );
        assert_eq!(before.integrity.archive_size, after.integrity.archive_size);
        assert_eq!(
            before.integrity.extracted_sha256,
            after.integrity.extracted_sha256
        );
        assert_eq!(
            before.integrity.extracted_size,
            after.integrity.extracted_size
        );
        // The two digests must not have collapsed into one column.
        assert_ne!(
            after.integrity.archive_sha256,
            after.integrity.extracted_sha256
        );
    }
}

#[test]
fn large_extracted_sizes_survive_the_signed_integer_boundary() {
    // The live catalog's desktop images are ~4 GiB, well past u32.
    let original = live_catalog(ProductScope::T3Only);
    let biggest = original
        .images
        .iter()
        .map(|image| image.integrity.extracted_size)
        .max()
        .expect("images exist");
    assert!(biggest > u64::from(u32::MAX), "fixture exercises >4 GiB");

    let mut store = T3CatalogStore::open_in_memory().expect("store opens");
    store
        .save(&original, ProductScope::T3Only, fetched_at())
        .expect("save succeeds");
    let loaded = store.load().expect("load succeeds").expect("catalog saved");

    let reloaded_biggest = loaded
        .images
        .iter()
        .map(|image| image.integrity.extracted_size)
        .max()
        .expect("images exist");
    assert_eq!(reloaded_biggest, biggest);
}

#[test]
fn the_same_url_may_legitimately_appear_under_more_than_one_group() {
    // The live catalog lists the four Ubuntu images twice: once at the top level and once inside
    // the "Ubuntu Images" sub-list. Both are real, selectable listings, so the store must not
    // collapse them — an image is identified by (url, group), not by url alone.
    let catalog = live_catalog(ProductScope::T3AndBeagleY);

    let mut urls: Vec<&str> = catalog.images.iter().map(|i| i.url.as_str()).collect();
    let total = urls.len();
    urls.sort_unstable();
    urls.dedup();
    assert!(
        urls.len() < total,
        "fixture is expected to contain duplicate URLs across groups"
    );

    let mut store = T3CatalogStore::open_in_memory().expect("store opens");
    store
        .save(&catalog, ProductScope::T3AndBeagleY, fetched_at())
        .expect("duplicate URLs across groups must be storable");

    let loaded = store.load().expect("load succeeds").expect("catalog saved");
    assert_eq!(loaded.images.len(), total);
}

#[test]
fn saving_twice_replaces_rather_than_duplicating() {
    let catalog = live_catalog(ProductScope::T3Only);
    let mut store = T3CatalogStore::open_in_memory().expect("store opens");

    store
        .save(&catalog, ProductScope::T3Only, fetched_at())
        .expect("first save");
    store
        .save(&catalog, ProductScope::T3Only, fetched_at())
        .expect("second save");

    let loaded = store.load().expect("load succeeds").expect("catalog saved");
    assert_eq!(loaded.images.len(), catalog.images.len());
    assert_eq!(loaded.boards.len(), catalog.boards.len());
}

#[test]
fn the_product_scope_the_catalog_was_validated_against_is_recorded() {
    let catalog = live_catalog(ProductScope::T3AndBeagleY);
    let mut store = T3CatalogStore::open_in_memory().expect("store opens");
    store
        .save(&catalog, ProductScope::T3AndBeagleY, fetched_at())
        .expect("save succeeds");

    assert_eq!(
        store.stored_scope().expect("scope readable"),
        Some(ProductScope::T3AndBeagleY)
    );
}

#[test]
fn reopening_an_existing_store_preserves_its_rows_and_does_not_recreate_the_schema() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("catalog.sqlite3");
    let catalog = live_catalog(ProductScope::T3Only);

    {
        let mut store = T3CatalogStore::open(&path).expect("store opens");
        store
            .save(&catalog, ProductScope::T3Only, fetched_at())
            .expect("save succeeds");
    }

    // A second open runs the migrator again; it must be a no-op, not a DROP-and-recreate.
    let store = T3CatalogStore::open(&path).expect("store reopens");
    assert_eq!(
        store.schema_version().expect("version readable"),
        CURRENT_SCHEMA_VERSION
    );
    let loaded = store
        .load()
        .expect("load succeeds")
        .expect("rows survived the reopen");
    assert_eq!(loaded.images.len(), catalog.images.len());
    assert_eq!(loaded.boards, catalog.boards);
}

#[test]
fn a_database_from_a_newer_build_is_refused_with_a_diagnostic() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("future.sqlite3");

    {
        let connection = rusqlite::Connection::open(&path).expect("raw connection");
        connection
            .pragma_update(None, "user_version", i64::from(CURRENT_SCHEMA_VERSION) + 5)
            .expect("stamp a future version");
    }

    let err = T3CatalogStore::open(&path).expect_err("a future schema must not be opened blindly");
    match err {
        StoreError::FutureSchema { found, supported } => {
            assert_eq!(found, CURRENT_SCHEMA_VERSION + 5);
            assert_eq!(supported, CURRENT_SCHEMA_VERSION);
        }
        other => panic!("expected FutureSchema, got {other:?}"),
    }
    // The message has to be actionable, not just "database error".
    assert!(err.to_string().contains("newer build"));
}

#[test]
fn a_file_that_is_not_a_database_fails_with_a_controlled_error() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("garbage.sqlite3");
    std::fs::write(&path, b"this is definitely not a sqlite database").expect("write garbage");

    let err = T3CatalogStore::open(&path).expect_err("a corrupt file must not open silently");
    // The point is that it is a typed error rather than a panic or a silent empty catalog.
    assert!(matches!(err, StoreError::Sqlite(_)));
}

#[test]
fn a_corrupt_hash_column_is_reported_instead_of_being_silently_accepted() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("corrupt.sqlite3");
    let catalog = live_catalog(ProductScope::T3Only);

    {
        let mut store = T3CatalogStore::open(&path).expect("store opens");
        store
            .save(&catalog, ProductScope::T3Only, fetched_at())
            .expect("save succeeds");
    }

    {
        let connection = rusqlite::Connection::open(&path).expect("raw connection");
        connection
            .execute("UPDATE images SET extracted_sha256 = 'deadbeef'", [])
            .expect("truncate a stored digest");
    }

    let store = T3CatalogStore::open(&path).expect("store reopens");
    let err = store
        .load()
        .expect_err("a truncated digest must not load as a valid image");
    match err {
        StoreError::Corrupt { table, reason } => {
            assert_eq!(table, "images");
            assert!(reason.contains("extracted_sha256"), "reason was: {reason}");
        }
        other => panic!("expected Corrupt, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Last-known-good behaviour (`instruction.md` §8.3)
// ---------------------------------------------------------------------------

const LIVE_BOOT_MANIFEST: &[u8] = include_bytes!("fixtures/t3/boot_manifest.json");
const BOOT_MANIFEST_URL: &str = "https://packages.t3gemstone.org/images/boot/t3-gem-o1/list.json";

fn required_artifacts() -> Vec<&'static str> {
    // The stage contract, not a hand-typed list.
    ["tiboot3.bin", "tispl.bin", "u-boot.img"].to_vec()
}

fn live_boot_manifest() -> bb_config::t3::VerifiedBootManifest {
    bb_config::t3::parse_boot_manifest(LIVE_BOOT_MANIFEST, &required_artifacts())
        .expect("the fixture manifest is complete")
}

#[test]
fn the_http_validators_of_the_stored_catalog_are_remembered() {
    let catalog = live_catalog(ProductScope::T3Only);
    let mut store = T3CatalogStore::open_in_memory().expect("store opens");

    let validators = HttpValidators {
        etag: Some("\"a1b2c3\"".to_owned()),
        last_modified: Some("Wed, 29 Jul 2026 10:00:00 GMT".to_owned()),
    };
    store
        .save_with_validators(&catalog, ProductScope::T3Only, fetched_at(), &validators)
        .expect("save succeeds");

    assert_eq!(
        store.stored_validators().expect("validators readable"),
        Some(validators)
    );
    assert_eq!(
        store.stored_fetched_at().expect("date readable"),
        Some(fetched_at())
    );
}

#[test]
fn a_catalog_saved_without_validators_reports_none_rather_than_empty_strings() {
    let catalog = live_catalog(ProductScope::T3Only);
    let mut store = T3CatalogStore::open_in_memory().expect("store opens");
    store
        .save(&catalog, ProductScope::T3Only, fetched_at())
        .expect("save succeeds");

    let validators = store
        .stored_validators()
        .expect("validators readable")
        .expect("a catalog was saved");
    assert!(validators.is_empty());
}

/// The point of the cache: a machine that starts up offline still has the catalog it validated
/// last time, on disk, across a restart.
#[test]
fn a_saved_catalog_survives_reopening_the_database() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let path = dir.path().join("catalog.sqlite3");
    let catalog = live_catalog(ProductScope::T3Only);

    {
        let mut store = T3CatalogStore::open(&path).expect("store opens");
        store
            .save(&catalog, ProductScope::T3Only, fetched_at())
            .expect("save succeeds");
    }

    let reopened = T3CatalogStore::open(&path).expect("store reopens");
    let loaded = reopened
        .load()
        .expect("load succeeds")
        .expect("the previous catalog is still there");
    assert_eq!(loaded.images, catalog.images);
}

/// A v1 database written by an older build must be carried forward, not discarded. If the upgrade
/// dropped the rows, the first launch after an update would present an empty catalog offline.
#[test]
fn a_v1_database_is_migrated_forward_with_its_rows_intact() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let path = dir.path().join("catalog.sqlite3");
    let catalog = live_catalog(ProductScope::T3Only);

    {
        // Build a v1 database by hand: schema v1 SQL, user_version pinned to 1.
        let conn = rusqlite::Connection::open(&path).expect("sqlite opens");
        conn.execute_batch(bb_config::t3::store::MIGRATIONS[0].1)
            .expect("v1 schema applies");
        conn.pragma_update(None, "user_version", 1i64)
            .expect("version set");
    }

    {
        // The current build opens it, migrates to v2, and writes a catalog.
        let mut store = T3CatalogStore::open(&path).expect("store migrates");
        assert_eq!(
            store.schema_version().expect("version readable"),
            CURRENT_SCHEMA_VERSION
        );
        store
            .save(&catalog, ProductScope::T3Only, fetched_at())
            .expect("save succeeds");
    }

    let reopened = T3CatalogStore::open(&path).expect("store reopens");
    assert_eq!(
        reopened
            .load()
            .expect("load succeeds")
            .expect("catalog present")
            .images
            .len(),
        catalog.images.len()
    );
}

#[test]
fn a_verified_boot_manifest_round_trips() {
    let mut store = T3CatalogStore::open_in_memory().expect("store opens");
    let manifest = live_boot_manifest();

    store
        .save_boot_manifest(
            "t3-gem-o1",
            BOOT_MANIFEST_URL,
            &manifest,
            fetched_at(),
            &HttpValidators::default(),
        )
        .expect("save succeeds");

    let loaded = store
        .load_boot_manifest("t3-gem-o1", &required_artifacts())
        .expect("load succeeds")
        .expect("a manifest was saved");

    assert_eq!(loaded, manifest);
    assert_eq!(loaded.artifacts()[0].name, "tiboot3.bin");
}

#[test]
fn a_board_with_no_stored_manifest_yields_nothing_to_start_dfu_with() {
    let store = T3CatalogStore::open_in_memory().expect("store opens");

    assert!(
        store
            .load_boot_manifest("t3-gem-o1", &required_artifacts())
            .expect("load succeeds")
            .is_none()
    );
}

/// If the stage contract grows an artifact the cached manifest never carried, the cache must fail
/// closed rather than hand back a boot chain that is one stage short.
#[test]
fn a_stored_manifest_that_no_longer_covers_the_stage_contract_is_refused() {
    let mut store = T3CatalogStore::open_in_memory().expect("store opens");
    store
        .save_boot_manifest(
            "t3-gem-o1",
            BOOT_MANIFEST_URL,
            &live_boot_manifest(),
            fetched_at(),
            &HttpValidators::default(),
        )
        .expect("save succeeds");

    let mut extended = required_artifacts();
    extended.push("a-future-stage.bin");

    let err = store
        .load_boot_manifest("t3-gem-o1", &extended)
        .expect_err("an incomplete manifest must not be returned");

    assert!(
        matches!(err, StoreError::Corrupt { table, .. } if table == "boot_manifest_artifacts"),
        "unexpected error: {err}"
    );
}
