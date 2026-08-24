use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RedirectRefusal {
    #[error("redirect chain exceeded the configured limit")]
    TooManyRedirects,
    #[error("redirect downgraded https to plaintext http")]
    Downgrade,
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("not a valid url: {0}")]
    InvalidUrl(String),

    #[error("refusing to fetch {url}: only https is allowed")]
    InsecureUrl {
        url: String,
    },

    #[error("{url} answered with HTTP {status}")]
    HttpStatus {
        url: String,
        status: u16,
    },

    #[error("{url}: {refusal}")]
    Redirect {
        url: String,
        refusal: RedirectRefusal,
    },

    #[error("transport failure for {url}: {source}")]
    Transport {
        url: String,
        source: reqwest::Error,
    },

    #[error("{url} sent more than the {limit} byte cap allows")]
    BodyTooLarge {
        url: String,
        limit: u64,
    },

    #[error("{url} delivered {actual} bytes but the catalog declares {expected}")]
    ArchiveSizeMismatch {
        url: String,
        expected: u64,
        actual: u64,
    },

    #[error("{url} delivered sha256 {actual} but the catalog declares {expected}")]
    ArchiveHashMismatch {
        url: String,
        expected: String,
        actual: String,
    },

    #[cfg(feature = "json")]
    #[error("could not decode {url} as json: {source}")]
    Json {
        url: String,
        source: serde_json::Error,
    },

    #[error("{context}: {source}")]
    Io {
        context: String,
        source: io::Error,
    },
}

impl DownloadError {
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
