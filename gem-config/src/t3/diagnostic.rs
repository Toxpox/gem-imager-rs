//! Typed, path-carrying diagnostics produced while validating a T3 catalog.
//!
//! `instruction.md` §6.2 forbids `VecSkipError`-style silent skipping in the T3 adapter: when an
//! entry is dropped, the caller must be able to say how many entries were rejected and why, and
//! the UI must be able to surface and log that. Every diagnostic therefore carries the JSON path
//! of the offending entry.

use std::fmt;

use crate::t3::sha256::Sha256ParseError;

/// What happened to the catalog entry a diagnostic refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    /// The entry was dropped from the canonical catalog and cannot be flashed.
    Rejected,
    /// The entry was kept, but with reduced capability or hidden from the product surface.
    Retained,
    /// The catalog as a whole is unusable.
    Fatal,
}

/// A single validation finding, always anchored to a JSON path such as
/// `os_list[4].subitems[2].extract_sha256`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum T3Diagnostic {
    /// A required field was absent.
    MissingField {
        /// JSON path of the entry.
        path: String,
        /// Name of the absent field.
        field: &'static str,
    },
    /// A hash field was present but not a valid 64-character digest.
    InvalidSha256 {
        /// JSON path of the entry.
        path: String,
        /// Name of the offending field.
        field: &'static str,
        /// Why parsing failed.
        error: Sha256ParseError,
    },
    /// `extract_size` was zero, so the extracted-size gate could never be meaningful.
    ZeroExtractSize {
        /// JSON path of the entry.
        path: String,
    },
    /// A URL field did not use HTTPS.
    InsecureUrl {
        /// JSON path of the entry.
        path: String,
        /// Name of the offending field.
        field: &'static str,
        /// Scheme actually seen.
        scheme: String,
    },
    /// A URL field could not be parsed at all.
    InvalidUrl {
        /// JSON path of the entry.
        path: String,
        /// Name of the offending field.
        field: &'static str,
        /// Raw value that failed to parse.
        value: String,
    },
    /// `release_date` was not an ISO-8601 calendar date.
    InvalidReleaseDate {
        /// JSON path of the entry.
        path: String,
        /// Raw value that failed to parse.
        value: String,
    },
    /// An image referenced a board tag that no device in the catalog declares.
    OrphanDeviceTag {
        /// JSON path of the entry.
        path: String,
        /// The unmatched tag.
        tag: String,
    },
    /// An image declared no board tags at all, so it can never be matched to a board.
    NoDeviceTags {
        /// JSON path of the entry.
        path: String,
    },
    /// A board parsed cleanly but is outside the configured product scope.
    ///
    /// The board is retained in the canonical model so a later scope decision does not require a
    /// re-parse (`instruction.md` §3.1), but it must not be shown on the product surface.
    OutOfProductScope {
        /// JSON path of the entry.
        path: String,
        /// Board name.
        board: String,
    },
    /// `init_format` held a value this build has no customization consumer for.
    ///
    /// The image is retained and remains flashable; only customization is disabled.
    UnsupportedInitFormat {
        /// JSON path of the entry.
        path: String,
        /// Raw value seen.
        value: String,
    },
    /// A board advertised `emmc: true` but is not the T3 board, so no verified DFU profile exists.
    ///
    /// Fail-closed: the board keeps SD support and loses DFU (`instruction.md` §6.2).
    EmmcWithoutVerifiedDfuProfile {
        /// JSON path of the entry.
        path: String,
        /// Board name.
        board: String,
    },
    /// A device entry declared no tags, so no image can ever match it.
    BoardWithoutTags {
        /// JSON path of the entry.
        path: String,
        /// Board name.
        board: String,
    },
    /// An icon URL was unusable, so the icon was dropped.
    ///
    /// Icons are cosmetic: dropping one must never remove a flashable board or image.
    DroppedIcon {
        /// JSON path of the entry.
        path: String,
        /// Why the icon was dropped.
        reason: String,
    },
}

impl T3Diagnostic {
    /// Whether the entry survived validation.
    pub fn severity(&self) -> DiagnosticSeverity {
        match self {
            Self::MissingField { .. }
            | Self::InvalidSha256 { .. }
            | Self::ZeroExtractSize { .. }
            | Self::InsecureUrl { .. }
            | Self::InvalidUrl { .. }
            | Self::InvalidReleaseDate { .. }
            | Self::OrphanDeviceTag { .. }
            | Self::NoDeviceTags { .. }
            | Self::BoardWithoutTags { .. } => DiagnosticSeverity::Rejected,
            Self::OutOfProductScope { .. }
            | Self::UnsupportedInitFormat { .. }
            | Self::EmmcWithoutVerifiedDfuProfile { .. }
            | Self::DroppedIcon { .. } => DiagnosticSeverity::Retained,
        }
    }

    /// JSON path of the entry this diagnostic refers to.
    pub fn path(&self) -> &str {
        match self {
            Self::MissingField { path, .. }
            | Self::InvalidSha256 { path, .. }
            | Self::ZeroExtractSize { path }
            | Self::InsecureUrl { path, .. }
            | Self::InvalidUrl { path, .. }
            | Self::InvalidReleaseDate { path, .. }
            | Self::OrphanDeviceTag { path, .. }
            | Self::NoDeviceTags { path }
            | Self::OutOfProductScope { path, .. }
            | Self::UnsupportedInitFormat { path, .. }
            | Self::EmmcWithoutVerifiedDfuProfile { path, .. }
            | Self::BoardWithoutTags { path, .. }
            | Self::DroppedIcon { path, .. } => path,
        }
    }

    /// Whether this diagnostic caused its entry to be dropped.
    pub fn is_rejection(&self) -> bool {
        self.severity() == DiagnosticSeverity::Rejected
    }
}

impl fmt::Display for T3Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingField { path, field } => {
                write!(f, "{path}: required field `{field}` is missing")
            }
            Self::InvalidSha256 { path, field, error } => {
                write!(f, "{path}.{field}: invalid SHA-256 ({error})")
            }
            Self::ZeroExtractSize { path } => {
                write!(f, "{path}.extract_size: must be greater than zero")
            }
            Self::InsecureUrl {
                path,
                field,
                scheme,
            } => write!(f, "{path}.{field}: expected https, got `{scheme}`"),
            Self::InvalidUrl { path, field, value } => {
                write!(f, "{path}.{field}: not a valid URL (`{value}`)")
            }
            Self::InvalidReleaseDate { path, value } => {
                write!(f, "{path}.release_date: not an ISO-8601 date (`{value}`)")
            }
            Self::OrphanDeviceTag { path, tag } => write!(
                f,
                "{path}.devices: tag `{tag}` matches no device in this catalog"
            ),
            Self::NoDeviceTags { path } => {
                write!(f, "{path}.devices: image declares no board tags")
            }
            Self::OutOfProductScope { path, board } => {
                write!(f, "{path}: board `{board}` is outside the product scope")
            }
            Self::UnsupportedInitFormat { path, value } => write!(
                f,
                "{path}.init_format: `{value}` has no customization consumer; \
                 customization disabled for this image"
            ),
            Self::EmmcWithoutVerifiedDfuProfile { path, board } => write!(
                f,
                "{path}: board `{board}` advertises eMMC but has no verified DFU profile; \
                 DFU disabled for this board"
            ),
            Self::BoardWithoutTags { path, board } => {
                write!(f, "{path}.tags: board `{board}` declares no tags")
            }
            Self::DroppedIcon { path, reason } => {
                write!(f, "{path}: icon dropped ({reason})")
            }
        }
    }
}

/// Summary of a validation run, for logging and for the UI's "partial catalog" banner.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiagnosticSummary {
    /// Entries dropped from the canonical catalog.
    pub rejected: usize,
    /// Entries kept with reduced capability or hidden from the product surface.
    pub retained: usize,
}

impl DiagnosticSummary {
    /// Count severities across a diagnostic list.
    pub fn of(diagnostics: &[T3Diagnostic]) -> Self {
        let mut summary = Self::default();
        for diagnostic in diagnostics {
            match diagnostic.severity() {
                DiagnosticSeverity::Rejected => summary.rejected += 1,
                DiagnosticSeverity::Retained => summary.retained += 1,
                DiagnosticSeverity::Fatal => {}
            }
        }
        summary
    }

    /// Whether anything at all was dropped.
    pub fn has_rejections(&self) -> bool {
        self.rejected > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_field_is_a_rejection_and_reports_its_path() {
        let diagnostic = T3Diagnostic::MissingField {
            path: "os_list[3]".to_owned(),
            field: "extract_sha256",
        };
        assert!(diagnostic.is_rejection());
        assert_eq!(diagnostic.path(), "os_list[3]");
        assert!(diagnostic.to_string().contains("extract_sha256"));
    }

    #[test]
    fn out_of_scope_board_is_retained_not_rejected() {
        let diagnostic = T3Diagnostic::OutOfProductScope {
            path: "imager.devices[2]".to_owned(),
            board: "BeagleY-AI".to_owned(),
        };
        assert!(!diagnostic.is_rejection());
        assert_eq!(diagnostic.severity(), DiagnosticSeverity::Retained);
    }

    #[test]
    fn unsupported_init_format_keeps_the_image_flashable() {
        let diagnostic = T3Diagnostic::UnsupportedInitFormat {
            path: "os_list[0]".to_owned(),
            value: "cloudinit".to_owned(),
        };
        assert_eq!(diagnostic.severity(), DiagnosticSeverity::Retained);
    }

    #[test]
    fn summary_counts_both_severities() {
        let diagnostics = vec![
            T3Diagnostic::ZeroExtractSize {
                path: "os_list[0]".to_owned(),
            },
            T3Diagnostic::OutOfProductScope {
                path: "imager.devices[2]".to_owned(),
                board: "BeagleY-AI".to_owned(),
            },
        ];
        let summary = DiagnosticSummary::of(&diagnostics);
        assert_eq!(summary.rejected, 1);
        assert_eq!(summary.retained, 1);
        assert!(summary.has_rejections());
    }
}
