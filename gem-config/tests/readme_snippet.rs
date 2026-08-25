use gem_config::t3::{ProductScope, T3_CATALOG_URL, catalog_to_config, parse_catalog};

#[test]
fn readme_usage_example_compiles_and_runs() {
    let bytes = include_bytes!("fixtures/t3/main_catalog.json");
    let parsed = parse_catalog(bytes, ProductScope::T3Only, T3_CATALOG_URL).unwrap();
    let config: gem_config::Config = catalog_to_config(&parsed.catalog);

    let json_config = serde_json::to_string_pretty(&config).unwrap();
    assert!(!json_config.is_empty());
}
