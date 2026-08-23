use gem_config::t3::{ProductScope, T3_CATALOG_URL, catalog_to_config, parse_catalog};

/// Mirrors the usage example in `gem-config/README.md` so the documented API cannot silently rot.
/// The README fetches `T3_CATALOG_URL` over HTTP; here the same bytes come from the checked-in
/// snapshot of that URL, since the crate has no HTTP client dependency.
#[test]
fn readme_usage_example_compiles_and_runs() {
    let bytes = include_bytes!("fixtures/t3/main_catalog.json");
    let parsed = parse_catalog(bytes, ProductScope::T3Only, T3_CATALOG_URL).unwrap();
    let config: gem_config::Config = catalog_to_config(&parsed.catalog);

    let json_config = serde_json::to_string_pretty(&config).unwrap();
    assert!(!json_config.is_empty());
}
