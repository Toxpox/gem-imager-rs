use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportPolicy {
    pub require_https: bool,
    pub connect_timeout: Duration,
    pub idle_timeout: Duration,
    pub metadata_timeout: Duration,
    pub stream_timeout: Duration,
    pub max_redirects: usize,
    pub max_metadata_body: u64,
    pub max_stream_body: u64,
}

impl TransportPolicy {
    pub const DEFAULT_MAX_METADATA_BODY: u64 = 8 * 1024 * 1024;

    pub const DEFAULT_MAX_STREAM_BODY: u64 = 32 * 1024 * 1024 * 1024;

    pub const fn secure() -> Self {
        Self {
            require_https: true,
            connect_timeout: Duration::from_secs(10),
            idle_timeout: Duration::from_secs(30),
            metadata_timeout: Duration::from_secs(60),
            stream_timeout: Duration::from_secs(6 * 60 * 60),
            max_redirects: 5,
            max_metadata_body: Self::DEFAULT_MAX_METADATA_BODY,
            max_stream_body: Self::DEFAULT_MAX_STREAM_BODY,
        }
    }

    pub const fn plaintext_for_tests() -> Self {
        Self {
            require_https: false,
            ..Self::secure()
        }
    }

    pub const fn with_max_redirects(mut self, max_redirects: usize) -> Self {
        self.max_redirects = max_redirects;
        self
    }

    pub const fn with_max_metadata_body(mut self, max_metadata_body: u64) -> Self {
        self.max_metadata_body = max_metadata_body;
        self
    }

    pub const fn with_max_stream_body(mut self, max_stream_body: u64) -> Self {
        self.max_stream_body = max_stream_body;
        self
    }
}

impl Default for TransportPolicy {
    fn default() -> Self {
        Self::secure()
    }
}
