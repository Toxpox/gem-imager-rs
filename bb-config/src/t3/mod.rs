//! Strict adapter for the T3 Gemstone image catalog.
//!
//! The remote document is never deserialized straight into GUI or database types. Three layers
//! sit between the network and the rest of the application (`instruction.md` §6.1):
//!
//! 1. [`raw`] — serde types close to the server schema. Almost everything is optional so a bad
//!    entry becomes a diagnostic instead of aborting the document.
//! 2. [`validate`] — enforces the §6.2 invariants and emits a [`diagnostic::T3Diagnostic`] with a
//!    JSON path for every entry it drops or downgrades.
//! 3. [`canonical`] — the model the application actually uses.
//!
//! The BeagleBoard adapter in [`crate::config`] is *not* reusable here: the T3 schema has no
//! `flasher` field, carries a separate `extract_sha256`, and expresses board capability through an
//! `emmc` boolean plus `matching_type`. It also relies on `VecSkipError`, which §6.2 forbids in
//! this adapter.
//!
//! ```no_run
//! use bb_config::t3::{parse_catalog, ProductScope};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let bytes: &[u8] = b"{}";
//! let parsed = parse_catalog(bytes, ProductScope::T3Only, "https://example.invalid/list.json")?;
//! for diagnostic in &parsed.diagnostics {
//!     eprintln!("{diagnostic}");
//! }
//! # Ok(())
//! # }
//! ```

pub mod boot_manifest;
pub mod bridge;
pub mod canonical;
pub mod diagnostic;
pub mod raw;
pub mod sha256;
#[cfg(feature = "store")]
pub mod store;
pub mod validate;

pub use boot_manifest::{
    BootArtifact, BootManifestError, VerifiedBootManifest, parse_boot_manifest,
};
pub use bridge::catalog_to_config;
pub use canonical::{
    BEAGLEY_BOARD_TAG, Board, BoardCapabilities, CustomizationProfile, DfuProfile, DfuStageSpec,
    Image, ImageIntegrity, MatchingType, ProductScope, T3_BOARD_TAG, T3_BOOT_MANIFEST_URL,
    T3_DFU_PRODUCT_ID, T3_DFU_RECONNECT_TIMEOUT, T3_DFU_VENDOR_ID, T3_RAW_EMMC_ALT_SETTING,
    T3InitFormat, WriteMethod,
};
pub use diagnostic::{DiagnosticSeverity, DiagnosticSummary, T3Diagnostic};
pub use raw::RawT3Catalog;
pub use sha256::{Sha256, Sha256ParseError};
#[cfg(feature = "store")]
pub use store::{CURRENT_SCHEMA_VERSION, StoreError, T3CatalogStore};
pub use validate::{
    CatalogProvenance, T3CatalogError, T3CatalogParse, ValidatedT3Catalog, parse_catalog,
    validate_catalog,
};

/// Canonical URL of the T3 image catalog (`instruction.md` §3.1).
pub const T3_CATALOG_URL: &str = "https://packages.t3gemstone.org/images/list.json";
