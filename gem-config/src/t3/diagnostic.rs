use std::fmt;

use crate::t3::sha256::Sha256ParseError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Rejected,
    Retained,
    Fatal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum T3Diagnostic {
    MissingField {
        path: String,
        field: &'static str,
    },
    InvalidSha256 {
        path: String,
        field: &'static str,
        error: Sha256ParseError,
    },
    ZeroExtractSize {
        path: String,
    },
    InsecureUrl {
        path: String,
        field: &'static str,
        scheme: String,
    },
    InvalidUrl {
        path: String,
        field: &'static str,
        value: String,
    },
    InvalidReleaseDate {
        path: String,
        value: String,
    },
    OrphanDeviceTag {
        path: String,
        tag: String,
    },
    NoDeviceTags {
        path: String,
    },
    OutOfProductScope {
        path: String,
        board: String,
    },
    UnsupportedInitFormat {
        path: String,
        value: String,
    },
    EmmcWithoutVerifiedDfuProfile {
        path: String,
        board: String,
    },
    BoardWithoutTags {
        path: String,
        board: String,
    },
    DroppedIcon {
        path: String,
        reason: String,
    },
}

impl T3Diagnostic {
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiagnosticSummary {
    pub rejected: usize,
    pub retained: usize,
}

impl DiagnosticSummary {
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
