//! A downloader for applications that must be able to prove what they downloaded.
//!
//! # Features
//!
//! - Async.
//! - Caches downloaded files in a directory on the filesystem, addressed by content hash.
//! - Verifies both the byte count and the SHA-256 of an archive before it is published to the
//!   cache, and publishes atomically (`instruction.md` §8.1, §8.2).
//! - Refuses plaintext transports, non-2xx responses, unbounded bodies and unbounded redirect
//!   chains by policy rather than by call site.
//! - Collapses concurrent downloads of the same archive into a single transfer.

mod error;
mod helpers;
mod policy;
mod single_flight;

/// Value sent as `User-Agent` on every request.
///
/// Product identity rather than crate identity: `packages.t3gemstone.org` sees the application
/// that is asking, not the internal crate name that happens to hold the HTTP client.
pub const USER_AGENT: &str = concat!("T3GemstoneImager/", env!("CARGO_PKG_VERSION"));

use helpers::sha256_from_path;
use single_flight::SingleFlight;

use futures_util::{StreamExt, TryStreamExt};
#[cfg(feature = "json")]
use serde::de::DeserializeOwned;
use sha2::{Digest as _, Sha256};
use std::{
    io,
    path::{Path, PathBuf},
};
use tokio::io::AsyncWriteExt;

pub use error::{DownloadError, RedirectRefusal};
pub use policy::TransportPolicy;
pub use reqwest::IntoUrl;

/// What the catalog promises about a compressed archive.
///
/// Two of the four gates of `instruction.md` §8.1. The extracted-side gates live with the decoder
/// in `bb-flasher`, because only the decoder sees the extracted bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveIntegrity {
    /// SHA-256 of the compressed archive. Always known; the catalog adapter requires it.
    pub sha256: [u8; 32],
    /// Byte count of the compressed archive, when the catalog publishes one.
    ///
    /// `Content-Length` is *not* used in its place: `instruction.md` §8.2 treats the header as an
    /// auxiliary hint, never as proof.
    pub size: Option<u64>,
}

impl ArchiveIntegrity {
    /// An archive whose size the catalog does not publish.
    pub const fn from_sha256(sha256: [u8; 32]) -> Self {
        Self { sha256, size: None }
    }

    /// An archive with both values published.
    pub const fn new(sha256: [u8; 32], size: u64) -> Self {
        Self {
            sha256,
            size: Some(size),
        }
    }
}

/// Downloader that caches files in the provided directory.
///
/// # Cache identity
///
/// Archives are addressed by their SHA-256, so a cache hit is by construction a hash match. Assets
/// with no published hash (board and image icons) are addressed by a digest *of their URL*; the two
/// namespaces are not interchangeable, and a URL-addressed entry can only be invalidated by
/// changing the URL.
///
/// # Thread safety
///
/// Clone freely: the HTTP clients and the in-flight table are shared internally.
#[derive(Debug, Clone)]
pub struct Downloader {
    /// Bounded, wall-clock-capped client for catalogs, manifests and icons.
    metadata_client: reqwest::Client,
    /// Long-lived client for archive streams; stalls are caught by the idle timeout.
    stream_client: reqwest::Client,
    cache_dir: PathBuf,
    policy: TransportPolicy,
    in_flight: SingleFlight,
}

impl Downloader {
    /// Create a downloader with the shipping [`TransportPolicy`].
    pub fn new<P: Into<PathBuf>>(cache_dir: P) -> io::Result<Self> {
        Self::with_policy(cache_dir, TransportPolicy::default())
    }

    /// Create a downloader with an explicit transport policy.
    pub fn with_policy<P: Into<PathBuf>>(
        cache_dir: P,
        policy: TransportPolicy,
    ) -> io::Result<Self> {
        let cache_dir = cache_dir.into();

        if !cache_dir.exists() {
            let _ = std::fs::create_dir_all(&cache_dir);
        }

        if cache_dir.exists() && !cache_dir.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "cache_dir should be a directory",
            ));
        }

        Ok(Self {
            metadata_client: build_client(&policy, policy.metadata_timeout)?,
            stream_client: build_client(&policy, policy.stream_timeout)?,
            cache_dir,
            policy,
            in_flight: SingleFlight::default(),
        })
    }

    /// The transport policy in force.
    pub const fn policy(&self) -> &TransportPolicy {
        &self.policy
    }

    /// Check whether a file with a particular SHA-256 is already cached.
    ///
    /// The cached file is re-hashed rather than trusted by name, and a file that no longer matches
    /// is evicted.
    pub fn check_cache_from_sha(&self, sha256: [u8; 32]) -> Option<PathBuf> {
        let file_path = self.path_from_sha(sha256);

        if file_path.exists() {
            if let Ok(hash) = sha256_from_path(&file_path)
                && hash == sha256
            {
                return Some(file_path);
            }

            // Delete old file
            let _ = std::fs::remove_file(&file_path);
        }

        None
    }

    /// Download a JSON document without caching it.
    ///
    /// The body is bounded by [`TransportPolicy::max_metadata_body`] before it is parsed, so a
    /// hostile or broken server cannot exhaust memory.
    #[cfg(feature = "json")]
    pub async fn download_json_no_cache<T, U>(&self, url: U) -> Result<T, DownloadError>
    where
        T: DeserializeOwned,
        U: reqwest::IntoUrl,
    {
        let url = self.check_url(url)?;
        let response = self.send(&self.metadata_client, &url).await?;
        let body = self
            .collect_bounded(&url, response, self.policy.max_metadata_body)
            .await?;

        serde_json::from_slice(&body).map_err(|source| DownloadError::Json {
            url: url.to_string(),
            source,
        })
    }

    /// Fetch a URL-addressed asset, returning the cached path.
    ///
    /// Used for assets the catalog publishes no hash for (icons). Because there is no hash to
    /// verify, this path proves nothing about the content beyond "the server sent it"; it is
    /// deliberately not used for anything that gets written to a board.
    pub async fn download<U: reqwest::IntoUrl>(&self, url: U) -> Result<PathBuf, DownloadError> {
        let url = self.check_url(url)?;
        let file_path = self.path_from_url(&url);

        // Check cache
        if file_path.exists() {
            return Ok(file_path);
        }

        let response = self.send(&self.metadata_client, &url).await?;
        let body = self
            .collect_bounded(&url, response, self.policy.max_metadata_body)
            .await?;

        publish_bytes(&file_path, body).await?;

        Ok(file_path)
    }

    /// Download an archive, verify it against `integrity`, and stream it to `writer` as it arrives.
    ///
    /// The caller can start decoding before the download finishes, which is why verification cannot
    /// be a post-hoc check on a finished file: the byte count and digest are computed over the same
    /// bytes that are handed to the reader, and a mismatch fails the whole operation. The verified
    /// content is only published to the cache after both gates pass, and the publish itself is
    /// atomic (`instruction.md` §8.2).
    ///
    /// Concurrent calls for the same digest are serialized; the loser of the race copies the
    /// now-cached bytes instead of fetching them again.
    pub async fn download_to_stream<U: reqwest::IntoUrl>(
        &self,
        url: U,
        integrity: ArchiveIntegrity,
        mut writer: bb_helper::file_stream::WriterFileStream,
    ) -> Result<(), DownloadError> {
        let url = self.check_url(url)?;
        tracing::debug!(
            "Download {:?} with sha256: {:?}",
            url,
            const_hex::encode(integrity.sha256)
        );

        let _slot = self.in_flight.acquire(integrity.sha256).await;
        let file_path = self.path_from_sha(integrity.sha256);

        // Re-checked inside the single-flight slot: another task may have published these exact
        // bytes while this one was waiting.
        if let Some(cached) = self.check_cache_from_sha(integrity.sha256) {
            tracing::info!("Serving archive from cache instead of downloading again");
            return copy_cached_to_writer(&cached, &mut writer).await;
        }

        let limit = integrity.size.unwrap_or(self.policy.max_stream_body);

        {
            let mut file = tokio::io::BufWriter::new(&mut writer);

            let response = self.send(&self.stream_client, &url).await?;
            let mut response_stream = response.bytes_stream().map_err(io::Error::other);

            let mut hasher = Sha256::new();
            let mut received: u64 = 0;

            while let Some(chunk) = response_stream.next().await {
                let mut data = chunk.map_err(|source| DownloadError::Io {
                    context: format!("reading the response body of {url}"),
                    source,
                })?;

                received += data.len() as u64;
                if received > limit {
                    return Err(if integrity.size.is_some() {
                        DownloadError::ArchiveSizeMismatch {
                            url: url.to_string(),
                            expected: limit,
                            actual: received,
                        }
                    } else {
                        DownloadError::BodyTooLarge {
                            url: url.to_string(),
                            limit,
                        }
                    });
                }

                hasher.update(&data);
                file.write_all_buf(&mut data)
                    .await
                    .map_err(|source| DownloadError::io("writing the download stream", source))?;
            }

            if let Some(expected) = integrity.size
                && received != expected
            {
                tracing::error!("Expected {expected} archive bytes, got {received}");
                return Err(DownloadError::ArchiveSizeMismatch {
                    url: url.to_string(),
                    expected,
                    actual: received,
                });
            }

            let hash: [u8; 32] = hasher
                .finalize()
                .as_slice()
                .try_into()
                .expect("SHA-256 is 32 bytes");

            if hash != integrity.sha256 {
                tracing::error!(
                    "Expected SHA256: {}, got {}",
                    const_hex::encode(integrity.sha256),
                    const_hex::encode(hash)
                );
                return Err(DownloadError::ArchiveHashMismatch {
                    url: url.to_string(),
                    expected: const_hex::encode(integrity.sha256),
                    actual: const_hex::encode(hash),
                });
            }

            file.flush()
                .await
                .map_err(|source| DownloadError::io("flushing the download stream", source))?;
        }

        tracing::info!("Publishing the verified download to the cache");
        writer
            .persist(&file_path)
            .await
            .map_err(|source| DownloadError::io("publishing the download to the cache", source))
    }

    /// Reject a URL the policy forbids before a single packet is sent.
    fn check_url<U: reqwest::IntoUrl>(&self, url: U) -> Result<reqwest::Url, DownloadError> {
        let url = url
            .into_url()
            .map_err(|err| DownloadError::InvalidUrl(err.to_string()))?;

        if self.policy.require_https && url.scheme() != "https" {
            return Err(DownloadError::InsecureUrl {
                url: url.to_string(),
            });
        }

        Ok(url)
    }

    /// Issue the request and accept only a 2xx answer.
    async fn send(
        &self,
        client: &reqwest::Client,
        url: &reqwest::Url,
    ) -> Result<reqwest::Response, DownloadError> {
        let response = client
            .get(url.clone())
            .send()
            .await
            .map_err(|source| DownloadError::from_reqwest(url, source))?;

        if !response.status().is_success() {
            return Err(DownloadError::HttpStatus {
                url: url.to_string(),
                status: response.status().as_u16(),
            });
        }

        Ok(response)
    }

    /// Read a whole body into memory, refusing to grow past `limit`.
    async fn collect_bounded(
        &self,
        url: &reqwest::Url,
        response: reqwest::Response,
        limit: u64,
    ) -> Result<Vec<u8>, DownloadError> {
        let mut stream = response.bytes_stream();
        let mut body: Vec<u8> = Vec::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|source| DownloadError::from_reqwest(url, source))?;

            if body.len() as u64 + chunk.len() as u64 > limit {
                return Err(DownloadError::BodyTooLarge {
                    url: url.to_string(),
                    limit,
                });
            }

            body.extend_from_slice(&chunk);
        }

        Ok(body)
    }

    fn path_from_url(&self, url: &reqwest::Url) -> PathBuf {
        let file_name: [u8; 32] = Sha256::new()
            .chain_update(url.as_str())
            .finalize()
            .as_slice()
            .try_into()
            .expect("SHA-256 is 32 bytes");
        let path = self.path_from_sha(file_name);

        match Path::new(url.path()).extension() {
            Some(ext) => path.with_extension(ext),
            None => path,
        }
    }

    fn path_from_sha(&self, sha256: [u8; 32]) -> PathBuf {
        let file_name = const_hex::encode(sha256);
        self.cache_dir.join(file_name)
    }
}

/// Build a client that enforces the policy's transport limits.
fn build_client(
    policy: &TransportPolicy,
    total_timeout: std::time::Duration,
) -> io::Result<reqwest::Client> {
    let max_redirects = policy.max_redirects;

    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(policy.connect_timeout)
        .read_timeout(policy.idle_timeout)
        .timeout(total_timeout)
        .redirect(reqwest::redirect::Policy::custom(
            move |attempt| match refuse_redirect(attempt.previous(), attempt.url(), max_redirects) {
                Some(refusal) => attempt.error(refusal),
                None => attempt.follow(),
            },
        ))
        .build()
        .map_err(io::Error::other)
}

/// Decide whether a redirect hop is allowed (`instruction.md` §8.2).
///
/// Split out of the client builder so both rules are directly testable: a live plaintext mock
/// server cannot produce an https-to-http hop, and a rule with no test is a rule that erodes.
fn refuse_redirect(
    previous: &[reqwest::Url],
    next: &reqwest::Url,
    max_redirects: usize,
) -> Option<RedirectRefusal> {
    if previous.len() >= max_redirects {
        return Some(RedirectRefusal::TooManyRedirects);
    }

    let downgraded = previous
        .last()
        .is_some_and(|previous| previous.scheme() == "https")
        && next.scheme() != "https";

    downgraded.then_some(RedirectRefusal::Downgrade)
}

/// Write `body` to `path` through a scratch file, so `path` never names a partial document.
async fn publish_bytes(path: &Path, body: Vec<u8>) -> Result<(), DownloadError> {
    let scratch = path.with_extension(format!("part-{}", std::process::id()));
    let scratch_for_task = scratch.clone();
    let target = path.to_path_buf();

    let result = tokio::task::spawn_blocking(move || {
        std::fs::write(&scratch_for_task, body)?;
        std::fs::rename(&scratch_for_task, &target)
    })
    .await
    .map_err(|err| DownloadError::io("joining the cache write task", io::Error::other(err)))?;

    if result.is_err() {
        let _ = tokio::fs::remove_file(&scratch).await;
    }

    result.map_err(|source| DownloadError::io("publishing the download to the cache", source))
}

/// Feed an already-verified cache entry to a waiting reader.
async fn copy_cached_to_writer(
    cached: &Path,
    writer: &mut bb_helper::file_stream::WriterFileStream,
) -> Result<(), DownloadError> {
    let mut file = tokio::fs::File::open(cached)
        .await
        .map_err(|source| DownloadError::io("opening the cached archive", source))?;

    tokio::io::copy(&mut file, writer)
        .await
        .map_err(|source| DownloadError::io("replaying the cached archive", source))?;

    writer
        .flush()
        .await
        .map_err(|source| DownloadError::io("flushing the cached archive", source))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to generate a 32-byte SHA256 array from a slice
    fn mock_sha256(data: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher.finalize().into()
    }

    #[test]
    fn test_downloader_new_creates_dir() {
        let tmp_dir = tempfile::TempDir::new().unwrap();
        let cache_dir = tmp_dir.path().join("nested_cache_dir");

        assert!(!cache_dir.exists());

        let downloader = Downloader::new(&cache_dir).unwrap();

        assert!(downloader.cache_dir.exists());
        assert!(downloader.cache_dir.is_dir());
    }

    #[test]
    fn test_check_cache_from_sha() {
        let tmp_dir = tempfile::TempDir::new().unwrap();
        let downloader = Downloader::new(tmp_dir.path()).unwrap();

        let content = b"Secure payload data";
        let sha = mock_sha256(content);
        let expected_path = downloader.path_from_sha(sha);

        // Scenario A: Check cache when empty -> Should return None
        assert!(downloader.check_cache_from_sha(sha).is_none());

        // Scenario B: Manually populate valid file into cache
        std::fs::write(&expected_path, content).unwrap();

        // Check cache -> Should return Some(PathBuf) matching expected path
        let cached_path = downloader.check_cache_from_sha(sha).unwrap();
        assert_eq!(cached_path, expected_path);

        // Scenario C: Corrupt the file to trigger invalidation
        std::fs::write(&expected_path, b"Tampered/Corrupted data").unwrap();

        // Check cache -> Should return None and evict/delete the corrupted file from disk
        assert!(downloader.check_cache_from_sha(sha).is_none());
        assert!(
            !expected_path.exists(),
            "Corrupted cache file should be scrubbed from disk"
        );
    }

    #[tokio::test]
    async fn plaintext_urls_are_refused_before_any_request() {
        let tmp_dir = tempfile::TempDir::new().unwrap();
        let downloader = Downloader::new(tmp_dir.path()).unwrap();

        let err = downloader
            .download("http://example.invalid/os.img.xz")
            .await
            .expect_err("http must be refused under the default policy");

        assert!(matches!(err, DownloadError::InsecureUrl { .. }));
    }

    #[test]
    fn url_addressed_entries_keep_their_extension_and_never_collide_with_hash_entries() {
        let tmp_dir = tempfile::TempDir::new().unwrap();
        let downloader = Downloader::new(tmp_dir.path()).unwrap();

        let icon = downloader.path_from_url(&"https://example.com/icons/t3.png".parse().unwrap());
        assert_eq!(icon.extension().unwrap(), "png");

        // A hash-addressed entry has no extension, so the two namespaces cannot alias.
        let archive = downloader.path_from_sha([3u8; 32]);
        assert!(archive.extension().is_none());
        assert_ne!(icon, archive);
    }

    #[test]
    fn a_redirect_that_drops_https_is_refused() {
        let previous = ["https://packages.t3gemstone.org/images/os.img.xz"
            .parse()
            .unwrap()];
        let next = "http://mirror.invalid/os.img.xz".parse().unwrap();

        assert_eq!(
            refuse_redirect(&previous, &next, 5),
            Some(RedirectRefusal::Downgrade)
        );
    }

    #[test]
    fn a_redirect_that_stays_on_https_is_followed() {
        let previous = ["https://packages.t3gemstone.org/images/os.img.xz"
            .parse()
            .unwrap()];
        let next = "https://mirror.example/os.img.xz".parse().unwrap();

        assert_eq!(refuse_redirect(&previous, &next, 5), None);
    }

    #[test]
    fn a_plaintext_chain_is_not_treated_as_a_downgrade() {
        // With `require_https` disabled the entry point may legitimately be http; only losing
        // https that we already had counts as a downgrade.
        let previous = ["http://localhost:8080/a".parse().unwrap()];
        let next = "http://localhost:8080/b".parse().unwrap();

        assert_eq!(refuse_redirect(&previous, &next, 5), None);
    }

    #[test]
    fn the_redirect_limit_is_inclusive_of_hops_already_taken() {
        let previous: Vec<reqwest::Url> = (0..5)
            .map(|i| format!("https://example.com/{i}").parse().unwrap())
            .collect();
        let next = "https://example.com/final".parse().unwrap();

        assert_eq!(
            refuse_redirect(&previous, &next, 5),
            Some(RedirectRefusal::TooManyRedirects)
        );
        assert_eq!(refuse_redirect(&previous[..4], &next, 5), None);
    }

    #[test]
    fn a_url_without_a_file_extension_is_still_cacheable() {
        let tmp_dir = tempfile::TempDir::new().unwrap();
        let downloader = Downloader::new(tmp_dir.path()).unwrap();

        // The pre-existing implementation panicked here.
        let path = downloader.path_from_url(&"https://example.com/icon".parse().unwrap());
        assert!(path.extension().is_none());
    }
}
