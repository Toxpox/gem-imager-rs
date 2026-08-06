# T3 test fixtures

These version-controlled fixtures contain no personal data.

`main_catalog.json` and `boot_manifest.json` are snapshots of the live service. The catalog tests also act as schema-drift checks: incompatible server changes cause the tests to fail.

## Files

- `main_catalog.json` — snapshot of `https://packages.t3gemstone.org/images/list.json` taken on 2026-07-31. It contains three device entries, including the tagless `No filtering` pseudo-device, and 34 leaf images split evenly between T3-GEM-O1 and BeagleY-AI.
- `boot_manifest.json` — snapshot of `https://packages.t3gemstone.org/images/boot/t3-gem-o1/list.json`. The service publishes only `files[].name` and `files[].sha256`; DFU alt-setting selection therefore comes from `DfuProfile::t3_gem_o1()` rather than server data.
- `invalid/missing_hash.json` — an image missing `extract_sha256` next to a valid image.
- `invalid/bad_url_downgrade.json` — an HTTP image URL next to a valid HTTPS image.
- `invalid/orphan_tag.json` — an orphan device tag, zero `extract_size`, and a 63-character hash next to a valid image.
- `dfu_contract.md` — the fixed T3 DFU behavior used by contract tests.

Every invalid fixture includes a valid neighboring entry to verify that one malformed item does not invalidate the entire catalog.

## Binary data

The fixtures do not contain OS images or boot artifacts. Tests generate small synthetic XZ and raw payloads when needed.