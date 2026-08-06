//! Transport rules every request in this crate obeys (`instruction.md` §8.2).
//!
//! These are deliberately *policy*, not per-call arguments: a rule that each caller can forget is
//! not a rule. [`TransportPolicy::default`] is the shipping configuration; the loosened
//! [`TransportPolicy::plaintext_for_tests`] exists so the test suite can talk to a local mock
//! server without turning https enforcement into an opt-in.

use std::time::Duration;

/// Transport limits applied to every request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportPolicy {
    /// Refuse any URL that is not `https`.
    pub require_https: bool,
    /// Cap on establishing the TCP/TLS connection.
    pub connect_timeout: Duration,
    /// Cap on the gap between two reads once the response is flowing.
    ///
    /// This is what stops a stalled server from hanging a multi-gigabyte image download forever;
    /// a wall-clock cap cannot do that job without also killing slow-but-healthy transfers.
    pub idle_timeout: Duration,
    /// Wall-clock cap on a metadata request (catalog, manifest, release info) end to end.
    pub metadata_timeout: Duration,
    /// Wall-clock cap on a streamed archive download end to end.
    pub stream_timeout: Duration,
    /// Maximum number of redirects to follow.
    pub max_redirects: usize,
    /// Maximum in-memory body size for metadata requests, in bytes.
    pub max_metadata_body: u64,
    /// Maximum streamed body size when the catalog publishes no archive size, in bytes.
    ///
    /// When a size *is* published it is the cap, because it is the tighter and authoritative one.
    pub max_stream_body: u64,
}

impl TransportPolicy {
    /// 8 MiB: the live T3 catalog is ~40 KiB and the boot manifest a few hundred bytes, so this is
    /// three orders of magnitude of headroom and still far from an out-of-memory risk.
    pub const DEFAULT_MAX_METADATA_BODY: u64 = 8 * 1024 * 1024;

    /// 32 GiB: larger than any plausible board image, small enough to bound a runaway response.
    pub const DEFAULT_MAX_STREAM_BODY: u64 = 32 * 1024 * 1024 * 1024;

    /// The shipping policy: https only, bounded in every dimension.
    pub const fn secure() -> Self {
        Self {
            require_https: true,
            connect_timeout: Duration::from_secs(10),
            idle_timeout: Duration::from_secs(30),
            metadata_timeout: Duration::from_secs(60),
            // Six hours covers a multi-gigabyte image on a slow link; the idle timeout is what
            // actually catches stalls.
            stream_timeout: Duration::from_secs(6 * 60 * 60),
            max_redirects: 5,
            max_metadata_body: Self::DEFAULT_MAX_METADATA_BODY,
            max_stream_body: Self::DEFAULT_MAX_STREAM_BODY,
        }
    }

    /// The shipping policy with https enforcement lifted, for tests against a local mock server.
    ///
    /// Every other limit is preserved, so tests still exercise the real timeout, redirect and body
    /// caps. Not intended for production use.
    pub const fn plaintext_for_tests() -> Self {
        Self {
            require_https: false,
            ..Self::secure()
        }
    }

    /// Same policy with a different redirect limit.
    pub const fn with_max_redirects(mut self, max_redirects: usize) -> Self {
        self.max_redirects = max_redirects;
        self
    }

    /// Same policy with a different metadata body cap.
    pub const fn with_max_metadata_body(mut self, max_metadata_body: u64) -> Self {
        self.max_metadata_body = max_metadata_body;
        self
    }

    /// Same policy with a different streamed body cap.
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
