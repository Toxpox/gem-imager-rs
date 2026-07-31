//! Persistence and migration tests for the T3 catalog store (`instruction.md` §6.4, §6.5).

use bb_config::t3::store::{CURRENT_SCHEMA_VERSION, StoreError, T3CatalogStore};
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
