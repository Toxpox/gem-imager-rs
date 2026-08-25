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

pub const T3_CATALOG_URL: &str = "https://packages.t3gemstone.org/images/list.json";
