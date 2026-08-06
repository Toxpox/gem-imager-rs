//! Integration tests for `Downloader::download_to_stream`, the streaming download path that owns
//! the two archive-side integrity gates of `instruction.md` §8.1, and for the transport rules of
//! §8.2. Together with the unit tests in `src/lib.rs` (scheme and redirect rules) these cover the
//! §8.4 matrix: wrong hash, short/long body, 404/500, redirect limit, cancellation leaving no
//! partial cache, two concurrent downloads of one hash, and a Unicode cache path.

use gem_downloader::{
    ArchiveIntegrity, DownloadError, Downloader, RedirectRefusal, TransportPolicy,
};
use gem_helper::file_stream::file_stream;
use httpmock::{Method::GET, MockServer};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// A downloader allowed to talk to the local plaintext mock server. Every other limit is the
/// shipping one.
fn test_downloader(cache: &Path) -> Downloader {
    Downloader::with_policy(cache, TransportPolicy::plaintext_for_tests()).unwrap()
}

/// Every entry currently in the cache directory.
fn entries(dir: &Path) -> Vec<PathBuf> {
    match std::fs::read_dir(dir) {
        Ok(dir) => dir.map(|e| e.unwrap().path()).collect(),
        Err(_) => Vec::new(),
    }
}

fn single_file(dir: &Path) -> Option<PathBuf> {
    entries(dir).into_iter().next()
}

#[tokio::test]
async fn download_to_stream_persists_on_sha_match() {
    let server = MockServer::start();
    let tmp = TempDir::new().unwrap();
    let downloader = test_downloader(tmp.path());

    let content = b"streamed payload bytes";
    let sha = sha256(content);

    let mock = server.mock(|when, then| {
        when.method(GET).path("/img");
        then.status(200).body(content);
    });

    let (writer, _reader) = file_stream().unwrap();
    downloader
        .download_to_stream(
            server.url("/img"),
            ArchiveIntegrity::new(sha, content.len() as u64),
            writer,
        )
        .await
        .expect("matching sha should succeed");

    mock.assert_calls(1);

    let path = single_file(tmp.path()).expect("a file should be persisted");
    assert_eq!(std::fs::read(&path).unwrap(), content);
    // Persisted under the SHA-derived cache name, with no scratch file left behind.
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        const_hex::encode(sha)
    );
    assert_eq!(entries(tmp.path()).len(), 1);
}

#[tokio::test]
async fn download_to_stream_rejects_sha_mismatch() {
    let server = MockServer::start();
    let tmp = TempDir::new().unwrap();
    let downloader = test_downloader(tmp.path());

    let content = b"streamed payload bytes";
    // A hash that cannot match the served content.
    let wrong_sha = [0u8; 32];

    let mock = server.mock(|when, then| {
        when.method(GET).path("/img");
        then.status(200).body(content);
    });

    let (writer, _reader) = file_stream().unwrap();
    let err = downloader
        .download_to_stream(
            server.url("/img"),
            ArchiveIntegrity::from_sha256(wrong_sha),
            writer,
        )
        .await
        .expect_err("mismatched sha must fail");

    mock.assert_calls(1);
    assert!(matches!(err, DownloadError::ArchiveHashMismatch { .. }));
    assert!(err.is_integrity_failure());
    assert!(
        single_file(tmp.path()).is_none(),
        "no file should be persisted when the checksum does not match"
    );
}

/// The archive size is an independent gate: an oversized body is refused mid-stream, before the
/// digest is even known.
#[tokio::test]
async fn a_body_longer_than_the_declared_archive_size_is_refused() {
    let server = MockServer::start();
    let tmp = TempDir::new().unwrap();
    let downloader = test_downloader(tmp.path());

    let content = b"twenty-four bytes long!!";
    let sha = sha256(content);

    server.mock(|when, then| {
        when.method(GET).path("/img");
        then.status(200).body(content);
    });

    let (writer, _reader) = file_stream().unwrap();
    let err = downloader
        .download_to_stream(server.url("/img"), ArchiveIntegrity::new(sha, 8), writer)
        .await
        .expect_err("an oversized body must fail even though the hash would have matched");

    assert!(matches!(err, DownloadError::ArchiveSizeMismatch { .. }));
    assert!(single_file(tmp.path()).is_none());
}

#[tokio::test]
async fn a_body_shorter_than_the_declared_archive_size_is_refused() {
    let server = MockServer::start();
    let tmp = TempDir::new().unwrap();
    let downloader = test_downloader(tmp.path());

    let content = b"short body";
    let sha = sha256(content);

    server.mock(|when, then| {
        when.method(GET).path("/img");
        then.status(200).body(content);
    });

    let (writer, _reader) = file_stream().unwrap();
    let err = downloader
        .download_to_stream(
            server.url("/img"),
            ArchiveIntegrity::new(sha, content.len() as u64 + 100),
            writer,
        )
        .await
        .expect_err("a truncated body must fail");

    match err {
        DownloadError::ArchiveSizeMismatch {
            expected, actual, ..
        } => {
            assert_eq!(expected, content.len() as u64 + 100);
            assert_eq!(actual, content.len() as u64);
        }
        other => panic!("expected a size mismatch, got {other}"),
    }
    assert!(single_file(tmp.path()).is_none());
}

#[tokio::test]
async fn a_404_body_is_never_mistaken_for_content() {
    let server = MockServer::start();
    let tmp = TempDir::new().unwrap();
    let downloader = test_downloader(tmp.path());

    let body = b"nope";
    server.mock(|when, then| {
        when.method(GET).path("/missing");
        then.status(404).body(body);
    });

    let (writer, _reader) = file_stream().unwrap();
    let err = downloader
        .download_to_stream(
            server.url("/missing"),
            // Deliberately the hash *of the error page*: only the status gate can catch this.
            ArchiveIntegrity::from_sha256(sha256(body)),
            writer,
        )
        .await
        .expect_err("404 must not be treated as content");

    assert!(matches!(err, DownloadError::HttpStatus { status: 404, .. }));
    assert!(single_file(tmp.path()).is_none());
}

#[tokio::test]
async fn a_500_is_not_a_download() {
    let server = MockServer::start();
    let tmp = TempDir::new().unwrap();
    let downloader = test_downloader(tmp.path());

    server.mock(|when, then| {
        when.method(GET).path("/boom");
        then.status(500).body("server on fire");
    });

    let (writer, _reader) = file_stream().unwrap();
    let err = downloader
        .download_to_stream(
            server.url("/boom"),
            ArchiveIntegrity::from_sha256([1u8; 32]),
            writer,
        )
        .await
        .expect_err("500 must fail");

    assert!(matches!(err, DownloadError::HttpStatus { status: 500, .. }));
}

#[tokio::test]
async fn a_redirect_chain_longer_than_the_limit_is_refused() {
    let server = MockServer::start();
    let tmp = TempDir::new().unwrap();
    let downloader = Downloader::with_policy(
        tmp.path(),
        TransportPolicy::plaintext_for_tests().with_max_redirects(2),
    )
    .unwrap();

    for hop in 0..6 {
        let next = server.url(format!("/hop{}", hop + 1));
        server.mock(move |when, then| {
            when.method(GET).path(format!("/hop{hop}"));
            then.status(302).header("location", next.clone());
        });
    }

    let (writer, _reader) = file_stream().unwrap();
    let err = downloader
        .download_to_stream(
            server.url("/hop0"),
            ArchiveIntegrity::from_sha256([2u8; 32]),
            writer,
        )
        .await
        .expect_err("an endless redirect chain must fail");

    assert!(
        matches!(
            err,
            DownloadError::Redirect {
                refusal: RedirectRefusal::TooManyRedirects,
                ..
            }
        ),
        "expected a redirect refusal, got {err}"
    );
}

#[tokio::test]
async fn a_cancelled_download_leaves_no_partial_cache_entry() {
    let server = MockServer::start();
    let tmp = TempDir::new().unwrap();
    let downloader = test_downloader(tmp.path());

    let content = vec![7u8; 512 * 1024];
    let sha = sha256(&content);

    server.mock(|when, then| {
        when.method(GET).path("/big");
        then.status(200)
            .delay(std::time::Duration::from_secs(5))
            .body(content.clone());
    });

    let (writer, _reader) = file_stream().unwrap();
    let task = {
        let downloader = downloader.clone();
        let url = server.url("/big");
        let len = content.len() as u64;
        tokio::spawn(async move {
            downloader
                .download_to_stream(url, ArchiveIntegrity::new(sha, len), writer)
                .await
        })
    };

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    task.abort();
    let _ = task.await;

    assert!(
        entries(tmp.path()).is_empty(),
        "an aborted download must leave neither a cache entry nor a scratch file: {:?}",
        entries(tmp.path())
    );
}

#[tokio::test]
async fn two_concurrent_downloads_of_the_same_hash_hit_the_network_once() {
    let server = MockServer::start();
    let tmp = TempDir::new().unwrap();
    let downloader = test_downloader(tmp.path());

    let content = b"the very same bytes";
    let sha = sha256(content);

    let mock = server.mock(|when, then| {
        when.method(GET).path("/img");
        then.status(200)
            .delay(std::time::Duration::from_millis(200))
            .body(content);
    });

    let mut tasks = Vec::new();
    for _ in 0..2 {
        let downloader = downloader.clone();
        let url = server.url("/img");
        let (writer, reader) = file_stream().unwrap();
        tasks.push(tokio::spawn(async move {
            let result = downloader
                .download_to_stream(
                    url,
                    ArchiveIntegrity::new(sha, content.len() as u64),
                    writer,
                )
                .await;
            // Keep the reader half alive for the duration of the write.
            drop(reader);
            result
        }));
    }

    for task in tasks {
        task.await.unwrap().expect("both callers must succeed");
    }

    mock.assert_calls(1);
    assert_eq!(entries(tmp.path()).len(), 1);
}

#[tokio::test]
async fn a_unicode_cache_path_works() {
    let server = MockServer::start();
    let tmp = TempDir::new().unwrap();
    // Turkish dotless i and Japanese katakana: both outside ASCII, both legal path components.
    let cache = tmp.path().join("önbellek-ışık").join("キャッシュ");
    let downloader = test_downloader(&cache);

    let content = "türkçe içerik".as_bytes();
    let sha = sha256(content);

    server.mock(|when, then| {
        when.method(GET).path("/img");
        then.status(200).body(content);
    });

    let (writer, _reader) = file_stream().unwrap();
    downloader
        .download_to_stream(
            server.url("/img"),
            ArchiveIntegrity::new(sha, content.len() as u64),
            writer,
        )
        .await
        .expect("a non-ascii cache directory must work");

    let path = single_file(&cache).expect("the file should be persisted under the unicode path");
    assert_eq!(std::fs::read(&path).unwrap(), content);
}

/// A second request for an already-cached hash must not touch the network, and must still deliver
/// the bytes to the caller's stream.
#[tokio::test]
async fn a_cached_archive_is_replayed_without_a_second_request() {
    let server = MockServer::start();
    let tmp = TempDir::new().unwrap();
    let downloader = test_downloader(tmp.path());

    let content = b"cache me once";
    let sha = sha256(content);
    let integrity = ArchiveIntegrity::new(sha, content.len() as u64);

    let mock = server.mock(|when, then| {
        when.method(GET).path("/img");
        then.status(200).body(content);
    });

    let (writer, _reader) = file_stream().unwrap();
    downloader
        .download_to_stream(server.url("/img"), integrity, writer)
        .await
        .unwrap();

    let (writer, mut reader) = file_stream().unwrap();
    let replayed = std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).unwrap();
        buf
    });

    downloader
        .download_to_stream(server.url("/img"), integrity, writer)
        .await
        .unwrap();

    mock.assert_calls(1);
    assert_eq!(
        replayed.join().unwrap(),
        content,
        "the cached bytes must reach the caller"
    );
}
