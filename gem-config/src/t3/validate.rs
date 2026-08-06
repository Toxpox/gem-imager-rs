//! Layer 2 of the T3 catalog adapter: enforce the `instruction.md` §6.2 invariants.
//!
//! Every rejection produces a typed diagnostic carrying a JSON path, and the caller is told how
//! many entries were dropped. Nothing is skipped silently — `VecSkipError` is deliberately absent
//! from this module.

use std::collections::BTreeSet;
use std::fmt;

use chrono::NaiveDate;
use url::Url;

use crate::t3::canonical::{
    Board, BoardCapabilities, CustomizationProfile, DfuProfile, Image, ImageIntegrity,
    MatchingType, ProductScope, T3_BOARD_TAG, T3InitFormat,
};
use crate::t3::diagnostic::T3Diagnostic;
use crate::t3::raw::{RawDevice, RawOsListItem, RawT3Catalog};
use crate::t3::sha256::Sha256;

/// Where a catalog came from, so a cached copy can be attributed later (`instruction.md` §6.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogProvenance {
    /// URL the document was fetched from.
    pub source_url: String,
    /// `imager.latest_version` as published, advisory only.
    pub latest_version: Option<String>,
}

/// A catalog that has passed every §6.2 invariant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedT3Catalog {
    /// Boards that parsed cleanly, including those outside the product scope.
    pub boards: Vec<Board>,
    /// Images that parsed cleanly.
    pub images: Vec<Image>,
    /// Where this catalog came from.
    pub provenance: CatalogProvenance,
}

impl ValidatedT3Catalog {
    /// Boards the product surface may show.
    pub fn boards_in_scope(&self) -> impl Iterator<Item = &Board> {
        self.boards.iter().filter(|board| board.in_product_scope)
    }

    /// Images that declare support for the mandatory T3 board.
    pub fn t3_images(&self) -> impl Iterator<Item = &Image> {
        self.images.iter().filter(|image| image.is_t3())
    }
}

/// Result of a successful parse, including everything that was dropped along the way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct T3CatalogParse {
    /// The validated catalog.
    pub catalog: ValidatedT3Catalog,
    /// Every finding, in document order.
    pub diagnostics: Vec<T3Diagnostic>,
    /// How many image entries were dropped.
    pub rejected_images: usize,
    /// How many device entries were dropped.
    pub rejected_boards: usize,
}

/// Why a catalog could not be used at all.
#[derive(Debug)]
pub enum T3CatalogError {
    /// The document was not valid JSON, or did not match the T3 shape.
    Json(serde_json::Error),
    /// No device entry survived validation.
    NoBoards {
        /// How many device entries were dropped.
        rejected: usize,
    },
    /// No image entry survived validation.
    ///
    /// `instruction.md` §6.2: a completely empty result must never be reported as success.
    NoUsableImages {
        /// How many image entries were dropped.
        rejected: usize,
    },
}

impl fmt::Display for T3CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(err) => write!(f, "catalog is not valid T3 JSON: {err}"),
            Self::NoBoards { rejected } => {
                write!(f, "catalog contains no usable board ({rejected} rejected)")
            }
            Self::NoUsableImages { rejected } => {
                write!(f, "catalog contains no usable image ({rejected} rejected)")
            }
        }
    }
}

impl std::error::Error for T3CatalogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(err) => Some(err),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for T3CatalogError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}

/// Parse and validate a T3 catalog document.
pub fn parse_catalog(
    bytes: &[u8],
    scope: ProductScope,
    source_url: &str,
) -> Result<T3CatalogParse, T3CatalogError> {
    let raw: RawT3Catalog = serde_json::from_slice(bytes)?;
    validate_catalog(raw, scope, source_url)
}

/// Validate an already-deserialized raw catalog.
pub fn validate_catalog(
    raw: RawT3Catalog,
    scope: ProductScope,
    source_url: &str,
) -> Result<T3CatalogParse, T3CatalogError> {
    let mut diagnostics = Vec::new();

    let (boards, rejected_boards) = validate_boards(&raw.imager.devices, scope, &mut diagnostics);
    if boards.is_empty() {
        return Err(T3CatalogError::NoBoards {
            rejected: rejected_boards,
        });
    }

    let known_tags: BTreeSet<&str> = boards
        .iter()
        .flat_map(|board| board.tags.iter().map(String::as_str))
        .collect();

    let (images, rejected_images) = validate_images(&raw.os_list, &known_tags, &mut diagnostics);
    if images.is_empty() {
        return Err(T3CatalogError::NoUsableImages {
            rejected: rejected_images,
        });
    }

    Ok(T3CatalogParse {
        catalog: ValidatedT3Catalog {
            boards,
            images,
            provenance: CatalogProvenance {
                source_url: source_url.to_owned(),
                latest_version: raw.imager.latest_version.clone(),
            },
        },
        diagnostics,
        rejected_images,
        rejected_boards,
    })
}

fn validate_boards(
    raw_devices: &[RawDevice],
    scope: ProductScope,
    diagnostics: &mut Vec<T3Diagnostic>,
) -> (Vec<Board>, usize) {
    let mut boards = Vec::new();
    let mut rejected = 0usize;

    for (index, raw) in raw_devices.iter().enumerate() {
        let path = format!("imager.devices[{index}]");
        match validate_board(raw, &path, scope) {
            Ok((board, notes)) => {
                diagnostics.extend(notes);
                boards.push(board);
            }
            Err(rejections) => {
                rejected += 1;
                diagnostics.extend(rejections);
            }
        }
    }

    (boards, rejected)
}

fn validate_board(
    raw: &RawDevice,
    path: &str,
    scope: ProductScope,
) -> Result<(Board, Vec<T3Diagnostic>), Vec<T3Diagnostic>> {
    let mut notes = Vec::new();

    let Some(name) = raw.name.as_deref().filter(|value| !value.is_empty()) else {
        return Err(vec![T3Diagnostic::MissingField {
            path: path.to_owned(),
            field: "name",
        }]);
    };
    let name = name.to_owned();

    let tags: BTreeSet<String> = raw.tags.iter().cloned().collect();
    if tags.is_empty() {
        // The live catalog carries a tagless "No filtering" pseudo-device; it can never match an
        // image, so it is not a board as far as this model is concerned.
        return Err(vec![T3Diagnostic::BoardWithoutTags {
            path: path.to_owned(),
            board: name,
        }]);
    }

    let icon = optional_icon(raw.icon.as_deref(), path, "icon", &mut notes);
    let capabilities = board_capabilities(raw.emmc, &tags, &name, path, &mut notes);

    let in_product_scope = scope.includes(&tags);
    if !in_product_scope {
        notes.push(T3Diagnostic::OutOfProductScope {
            path: path.to_owned(),
            board: name.clone(),
        });
    }

    Ok((
        Board {
            name,
            description: raw.description.clone().unwrap_or_default(),
            tags,
            icon,
            capabilities,
            matching_type: raw
                .matching_type
                .as_deref()
                .map(MatchingType::parse)
                .unwrap_or_default(),
            is_default: raw.is_default,
            in_product_scope,
        },
        notes,
    ))
}

/// Derive write capability from `emmc` plus the compile-time verified DFU profile.
///
/// `instruction.md` §6.2: `emmc: true` may only produce a *verified* T3 DFU profile. A non-T3
/// board claiming eMMC keeps SD and loses DFU, because no boot manifest exists for it.
fn board_capabilities(
    emmc: bool,
    tags: &BTreeSet<String>,
    name: &str,
    path: &str,
    notes: &mut Vec<T3Diagnostic>,
) -> BoardCapabilities {
    // Every board in the T3 catalog boots from SD; the schema has no per-board SD flag.
    let mut capabilities = BoardCapabilities::sd_only();

    if !emmc {
        return capabilities;
    }

    if tags.iter().any(|tag| tag == T3_BOARD_TAG) {
        capabilities.emmc_dfu = Some(DfuProfile::t3_gem_o1());
    } else {
        notes.push(T3Diagnostic::EmmcWithoutVerifiedDfuProfile {
            path: path.to_owned(),
            board: name.to_owned(),
        });
    }

    capabilities
}

/// Outcome of validating one image entry: either the image plus retained notes, or the rejections.
type ImageResult = Result<(Image, Vec<T3Diagnostic>), Vec<T3Diagnostic>>;

fn validate_images(
    raw_items: &[RawOsListItem],
    known_tags: &BTreeSet<&str>,
    diagnostics: &mut Vec<T3Diagnostic>,
) -> (Vec<Image>, usize) {
    let mut images = Vec::new();
    let mut rejected = 0usize;

    for (index, raw) in raw_items.iter().enumerate() {
        let path = format!("os_list[{index}]");

        match raw.subitems.as_deref() {
            None => {
                let result = validate_image(raw, &path, None, known_tags);
                push_image(result, diagnostics, &mut images, &mut rejected);
            }
            Some(subitems) => {
                for (sub_index, sub_raw) in subitems.iter().enumerate() {
                    let sub_path = format!("{path}.subitems[{sub_index}]");
                    let result = validate_image(sub_raw, &sub_path, raw.name.clone(), known_tags);
                    push_image(result, diagnostics, &mut images, &mut rejected);
                }
            }
        }
    }

    (images, rejected)
}

fn push_image(
    result: ImageResult,
    diagnostics: &mut Vec<T3Diagnostic>,
    images: &mut Vec<Image>,
    rejected: &mut usize,
) {
    match result {
        Ok((image, notes)) => {
            diagnostics.extend(notes);
            images.push(image);
        }
        Err(rejections) => {
            *rejected += 1;
            diagnostics.extend(rejections);
        }
    }
}

fn validate_image(
    raw: &RawOsListItem,
    path: &str,
    group: Option<String>,
    known_tags: &BTreeSet<&str>,
) -> ImageResult {
    let mut rejections = Vec::new();

    let name = require_str(raw.name.as_deref(), path, "name", &mut rejections);
    let url = require_https_url(raw.url.as_deref(), path, "url", &mut rejections);
    let archive_sha256 = require_hash(
        raw.image_download_sha256.as_deref(),
        path,
        "image_download_sha256",
        &mut rejections,
    );
    let extracted_sha256 = require_hash(
        raw.extract_sha256.as_deref(),
        path,
        "extract_sha256",
        &mut rejections,
    );
    let extracted_size = require_extract_size(raw.extract_size, path, &mut rejections);
    let release_date = require_release_date(raw.release_date.as_deref(), path, &mut rejections);
    let devices = require_device_tags(&raw.devices, path, known_tags, &mut rejections);

    // Every `None` above pushed a rejection, so this destructuring cannot panic and needs no
    // `unwrap`/`expect`.
    let (
        Some(name),
        Some(url),
        Some(archive_sha256),
        Some(extracted_sha256),
        Some(extracted_size),
        Some(release_date),
        Some(devices),
    ) = (
        name,
        url,
        archive_sha256,
        extracted_sha256,
        extracted_size,
        release_date,
        devices,
    )
    else {
        return Err(rejections);
    };

    let mut notes = Vec::new();
    let customization = customization_profile(raw.init_format.as_deref(), &name, path, &mut notes);
    let icon = optional_icon(raw.icon.as_deref(), path, "icon", &mut notes);

    Ok((
        Image {
            name,
            description: raw.description.clone().unwrap_or_default(),
            icon,
            url,
            integrity: ImageIntegrity {
                archive_sha256,
                archive_size: raw.image_download_size,
                extracted_sha256,
                extracted_size,
            },
            release_date,
            devices,
            customization,
            group,
        },
        notes,
    ))
}

fn customization_profile(
    raw: Option<&str>,
    image_name: &str,
    path: &str,
    notes: &mut Vec<T3Diagnostic>,
) -> Option<CustomizationProfile> {
    let value = raw.unwrap_or_default();
    match T3InitFormat::parse(value) {
        Some(init_format) => Some(CustomizationProfile {
            init_format,
            desktop_variant: image_name.to_lowercase().contains("desktop"),
        }),
        None => {
            notes.push(T3Diagnostic::UnsupportedInitFormat {
                path: path.to_owned(),
                value: value.to_owned(),
            });
            None
        }
    }
}

fn require_str(
    raw: Option<&str>,
    path: &str,
    field: &'static str,
    rejections: &mut Vec<T3Diagnostic>,
) -> Option<String> {
    match raw.filter(|value| !value.is_empty()) {
        Some(value) => Some(value.to_owned()),
        None => {
            rejections.push(T3Diagnostic::MissingField {
                path: path.to_owned(),
                field,
            });
            None
        }
    }
}

fn require_https_url(
    raw: Option<&str>,
    path: &str,
    field: &'static str,
    rejections: &mut Vec<T3Diagnostic>,
) -> Option<Url> {
    let Some(raw) = raw.filter(|value| !value.is_empty()) else {
        rejections.push(T3Diagnostic::MissingField {
            path: path.to_owned(),
            field,
        });
        return None;
    };

    match parse_https_url(raw, path, field) {
        Ok(url) => Some(url),
        Err(diagnostic) => {
            rejections.push(diagnostic);
            None
        }
    }
}

fn parse_https_url(raw: &str, path: &str, field: &'static str) -> Result<Url, T3Diagnostic> {
    let url = Url::parse(raw).map_err(|_| T3Diagnostic::InvalidUrl {
        path: path.to_owned(),
        field,
        value: raw.to_owned(),
    })?;

    if url.scheme() != "https" {
        return Err(T3Diagnostic::InsecureUrl {
            path: path.to_owned(),
            field,
            scheme: url.scheme().to_owned(),
        });
    }

    Ok(url)
}

/// Icons are cosmetic: a bad icon drops the icon and keeps the entry, rather than removing a
/// flashable image or board over a decorative field.
fn optional_icon(
    raw: Option<&str>,
    path: &str,
    field: &'static str,
    notes: &mut Vec<T3Diagnostic>,
) -> Option<Url> {
    let raw = raw.filter(|value| !value.is_empty())?;
    match parse_https_url(raw, path, field) {
        Ok(url) => Some(url),
        Err(diagnostic) => {
            notes.push(T3Diagnostic::DroppedIcon {
                path: path.to_owned(),
                reason: diagnostic.to_string(),
            });
            None
        }
    }
}

fn require_hash(
    raw: Option<&str>,
    path: &str,
    field: &'static str,
    rejections: &mut Vec<T3Diagnostic>,
) -> Option<Sha256> {
    let Some(raw) = raw.filter(|value| !value.is_empty()) else {
        rejections.push(T3Diagnostic::MissingField {
            path: path.to_owned(),
            field,
        });
        return None;
    };

    match Sha256::parse(raw) {
        Ok(hash) => Some(hash),
        Err(error) => {
            rejections.push(T3Diagnostic::InvalidSha256 {
                path: path.to_owned(),
                field,
                error,
            });
            None
        }
    }
}

fn require_extract_size(
    raw: Option<u64>,
    path: &str,
    rejections: &mut Vec<T3Diagnostic>,
) -> Option<u64> {
    match raw {
        Some(0) => {
            rejections.push(T3Diagnostic::ZeroExtractSize {
                path: path.to_owned(),
            });
            None
        }
        Some(size) => Some(size),
        None => {
            rejections.push(T3Diagnostic::MissingField {
                path: path.to_owned(),
                field: "extract_size",
            });
            None
        }
    }
}

fn require_release_date(
    raw: Option<&str>,
    path: &str,
    rejections: &mut Vec<T3Diagnostic>,
) -> Option<NaiveDate> {
    let Some(raw) = raw.filter(|value| !value.is_empty()) else {
        rejections.push(T3Diagnostic::MissingField {
            path: path.to_owned(),
            field: "release_date",
        });
        return None;
    };

    match raw.parse::<NaiveDate>() {
        Ok(date) => Some(date),
        Err(_) => {
            rejections.push(T3Diagnostic::InvalidReleaseDate {
                path: path.to_owned(),
                value: raw.to_owned(),
            });
            None
        }
    }
}

fn require_device_tags(
    raw: &[String],
    path: &str,
    known_tags: &BTreeSet<&str>,
    rejections: &mut Vec<T3Diagnostic>,
) -> Option<BTreeSet<String>> {
    if raw.is_empty() {
        rejections.push(T3Diagnostic::NoDeviceTags {
            path: path.to_owned(),
        });
        return None;
    }

    let mut orphaned = false;
    for tag in raw {
        if !known_tags.contains(tag.as_str()) {
            orphaned = true;
            rejections.push(T3Diagnostic::OrphanDeviceTag {
                path: path.to_owned(),
                tag: tag.clone(),
            });
        }
    }

    if orphaned {
        return None;
    }

    Some(raw.iter().cloned().collect())
}
