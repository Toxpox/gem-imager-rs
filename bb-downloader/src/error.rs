//! Typed download failures.
//!
//! `instruction.md` §8.1 makes the four integrity values independent hard gates, and §8.3 asks the
//! UI to tell "we are offline, showing the previous catalog" apart from "what the server sent is
//! wrong". Both need the *reason* a download failed, which a bare [`std::io::Error`] string cannot
//! carry. The [`From`] conversion below keeps the existing `io::Result` call sites compiling.

use std::io;

/// Why a redirect was refused.
///
/// This is a separate error type so it survives being boxed into a [`reqwest::Error`] by the
/// redirect policy and can be recovered from the source chain (see [`DownloadError::from_reqwest`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RedirectRefusal {
    /// The chain was longer than the configured limit.
    #[error("redirect chain exceeded the configured limit")]
    TooManyRedirects,
    /// A redirect moved from `https` to plaintext `http`.
    #[error("redirect downgraded https to plaintext http")]
    Downgrade,
}

/// Everything that can go wrong in this crate.
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    /// The string was not a URL at all.
    #[error("not a valid url: {0}")]
    InvalidUrl(String),

    /// The URL used a scheme other than `https` while the policy required transport security.
    #[error("refusing to fetch {url}: only https is allowed")]
    InsecureUrl {
        /// The rejected URL.
        url: String,
    },

    /// The response arrived, but not with a 2xx status.
    #[error("{url} answered with HTTP {status}")]
    HttpStatus {
        /// URL that was requested.
        url: String,
        /// Status code the server returned.
        status: u16,
    },

    /// A redirect was refused by policy.
    #[error("{url}: {refusal}")]
    Redirect {
        /// URL that was requested.
        url: String,
        /// Which rule refused it.
        refusal: RedirectRefusal,
    },

    /// Connect, TLS, timeout or mid-stream transport failure.
    #[error("transport failure for {url}: {source}")]
    Transport {
        /// URL that was requested.
        url: String,
        /// Underlying `reqwest` failure.
        source: reqwest::Error,
    },

    /// The body was larger than the configured cap.
    ///
    /// Applies to catalog and manifest bodies, which are held in memory, and to image archives
    /// whose declared size is exceeded mid-stream.
    #[error("{url} sent more than the {limit} byte cap allows")]
    BodyTooLarge {
        /// URL that was requested.
        url: String,
        /// Cap that was exceeded, in bytes.
        limit: u64,
    },

    /// The archive byte count did not match the catalog.
    ///
    /// One of the four gates of `instruction.md` §8.1; never downgraded to a warning.
    #[error("{url} delivered {actual} bytes but the catalog declares {expected}")]
    ArchiveSizeMismatch {
        /// URL that was requested.
        url: String,
        /// Size the catalog published.
        expected: u64,
        /// Size actually received.
        actual: u64,
    },

    /// The archive digest did not match the catalog.
    #[error("{url} delivered sha256 {actual} but the catalog declares {expected}")]
    ArchiveHashMismatch {
        /// URL that was requested.
        url: String,
        /// Digest the catalog published, hex encoded.
        expected: String,
        /// Digest actually received, hex encoded.
        actual: String,
    },

    /// The body was not the JSON the caller asked for.
    #[cfg(feature = "json")]
    #[error("could not decode {url} as json: {source}")]
    Json {
        /// URL that was requested.
        url: String,
        /// Underlying parse failure.
        source: serde_json::Error,
    },

    /// A filesystem operation around the cache failed.
    #[error("{context}: {source}")]
    Io {
        /// What was being attempted.
        context: String,
        /// Underlying failure.
        source: io::Error,
    },
}

impl DownloadError {
    /// Classify a `reqwest` failure, recovering a [`RedirectRefusal`] from its source chain.
    pub(crate) fn from_reqwest(url: &reqwest::Url, source: reqwest::Error) -> Self {
        let mut current: Option<&(dyn std::error::Error + 'static)> = Some(&source);
        while let Some(err) = current {
            if let Some(refusal) = err.downcast_ref::<RedirectRefusal>() {
                return Self::Redirect {
                    url: url.to_string(),
                    refusal: *refusal,
                };
            }
            current = err.source();
        }

        Self::Transport {
            url: url.to_string(),
            source,
        }
    }

    pub(crate) fn io(context: impl Into<String>, source: io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }

    /// Whether the failure means "the bytes we got are not the bytes the catalog promised".
    ///
    /// A caller may retry or fall back to a cached copy on a transport failure; it must never do so
    /// on an integrity failure.
    pub const fn is_integrity_failure(&self) -> bool {
        matches!(
            self,
            Self::ArchiveSizeMismatch { .. } | Self::ArchiveHashMismatch { .. }
        )
    }
}

impl From<DownloadError> for io::Error {
    fn from(value: DownloadError) -> Self {
        let kind = match &value {
            // Integrity mismatches keep `InvalidInput`: that is the kind the pre-existing
            // hash-mismatch contract returned and callers already match on it.
            DownloadError::InvalidUrl(_)
            | DownloadError::InsecureUrl { .. }
            | DownloadError::ArchiveSizeMismatch { .. }
            | DownloadError::ArchiveHashMismatch { .. } => io::ErrorKind::InvalidInput,
            DownloadError::HttpStatus { .. } | DownloadError::Redirect { .. } => {
                io::ErrorKind::InvalidData
            }
            DownloadError::BodyTooLarge { .. } => io::ErrorKind::FileTooLarge,
            #[cfg(feature = "json")]
            DownloadError::Json { .. } => io::ErrorKind::InvalidData,
            DownloadError::Transport { source, .. } if source.is_timeout() => {
                io::ErrorKind::TimedOut
            }
            DownloadError::Transport { .. } => io::ErrorKind::Other,
            DownloadError::Io { source, .. } => source.kind(),
        };

        Self::new(kind, value.to_string())
    }
}
