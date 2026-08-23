# gem-config

T3 Gemstone publishes a json catalog listing every board image, which the imager reads to get the latest images for each board. See the `t3` module for that schema and its strict validator.

The `config` module additionally parses the upstream BeagleBoard.org `distros.json` schema this project was derived from; `t3::bridge` adapts the T3 catalog onto it.

# Usage

Fetch the T3 catalog and bridge it onto the `Config` the front-ends consume:

```rust
use gem_config::t3::{catalog_to_config, parse_catalog, ProductScope, T3_CATALOG_URL};

let bytes = reqwest::blocking::get(T3_CATALOG_URL).unwrap().bytes().unwrap();
let parsed = parse_catalog(&bytes, ProductScope::T3Only, T3_CATALOG_URL).unwrap();
let config: gem_config::Config = catalog_to_config(&parsed.catalog);

// Convert back to JSON
let json_config = serde_json::to_string_pretty(&config).unwrap();
```
