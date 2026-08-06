use std::io;

use gem_downloader::{DownloadError, Downloader, TransportPolicy};
use httpmock::{Method::GET, MockServer};
use tempfile::TempDir;

/// A downloader allowed to talk to the local plaintext mock server.
fn test_downloader(cache: &std::path::Path) -> Downloader {
    Downloader::with_policy(cache, TransportPolicy::plaintext_for_tests()).unwrap()
}

#[test]
fn test_downloader_new_fails_if_path_is_file() {
    let tmp_dir = TempDir::new().unwrap();
    let file_path = tmp_dir.path().join("some_file.txt");
    std::fs::write(&file_path, "I am a file, not a directory").unwrap();

    let result = Downloader::new(&file_path);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::NotADirectory);
}

#[tokio::test]
async fn test_download_and_cache_by_url() {
    // Start a standalone mock server
    let server = MockServer::start();
    let tmp_dir = TempDir::new().unwrap();
    let downloader = test_downloader(tmp_dir.path());

    let file_content = b"Hello from httpmock!";

    // Setup the mock endpoint
    let download_mock = server.mock(|when, then| {
        when.method(GET).path("/file.txt");
        then.status(200)
            .header("content-type", "text/plain")
            .body(file_content);
    });

    let url = server.url("/file.txt");

    download_mock.assert_calls(0);

    // 1. First download (Cache Miss) -> Hits the mock server
    let path = downloader.download(&url).await.unwrap();
    assert!(path.exists());

    let saved_content = std::fs::read(&path).unwrap();
    assert_eq!(saved_content, file_content);
    download_mock.assert_calls(1);

    // No scratch file survives the publish.
    assert_eq!(std::fs::read_dir(tmp_dir.path()).unwrap().count(), 1);

    // 2. Second download (Cache Hit)
    let cached_path = downloader.download(&url).await.unwrap();
    assert_eq!(path, cached_path);

    // The mock hits should STILL be 1, proving it was pulled completely from the cache
    download_mock.assert_calls(1);
}

#[tokio::test]
async fn an_error_status_never_becomes_a_cached_asset() {
    let server = MockServer::start();
    let tmp_dir = TempDir::new().unwrap();
    let downloader = test_downloader(tmp_dir.path());

    server.mock(|when, then| {
        when.method(GET).path("/icon.png");
        then.status(503).body("upstream unavailable");
    });

    let err = downloader
        .download(server.url("/icon.png"))
        .await
        .expect_err("a 503 must not be cached as an icon");

    assert!(matches!(err, DownloadError::HttpStatus { status: 503, .. }));
    assert_eq!(std::fs::read_dir(tmp_dir.path()).unwrap().count(), 0);
}

#[cfg(feature = "json")]
#[tokio::test]
async fn test_download_json_no_cache() {
    use serde::Deserialize;

    #[derive(Deserialize, Debug, PartialEq)]
    struct TestData {
        status: String,
        code: u32,
    }

    let server = MockServer::start();
    let tmp_dir = TempDir::new().unwrap();
    let downloader = test_downloader(tmp_dir.path());

    let json_mock = server.mock(|when, then| {
        when.method(GET).path("/api/status");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"status": "ok", "code": 200}"#);
    });

    let url = server.url("/api/status");
    let result: TestData = downloader.download_json_no_cache(&url).await.unwrap();

    assert_eq!(
        result,
        TestData {
            status: "ok".to_string(),
            code: 200
        }
    );
    json_mock.assert_calls(1);

    // Ensure nothing was cached to disk during no-cache execution
    let entries = std::fs::read_dir(tmp_dir.path()).unwrap().count();
    assert_eq!(entries, 0);
}

#[cfg(feature = "json")]
#[tokio::test]
async fn test_download_json_no_cache_rejects_malformed_json() {
    #[derive(serde::Deserialize, Debug)]
    #[allow(dead_code)]
    struct TestData {
        status: String,
    }

    let server = MockServer::start();
    let tmp_dir = TempDir::new().unwrap();
    let downloader = test_downloader(tmp_dir.path());

    let mock = server.mock(|when, then| {
        when.method(GET).path("/api/bad");
        then.status(200)
            .header("content-type", "application/json")
            .body("{ this is not valid json ");
    });

    let result: Result<TestData, DownloadError> = downloader
        .download_json_no_cache(&server.url("/api/bad"))
        .await;

    assert!(
        result.is_err(),
        "malformed JSON body should surface an error"
    );
    mock.assert_calls(1);
}

/// `instruction.md` §8.2 requires a maximum body size for catalog and manifest documents; without
/// it a hostile or broken server can grow the client's memory without bound.
#[cfg(feature = "json")]
#[tokio::test]
async fn an_oversized_catalog_body_is_refused_before_it_is_parsed() {
    #[derive(serde::Deserialize, Debug)]
    #[allow(dead_code)]
    struct TestData {
        status: String,
    }

    let server = MockServer::start();
    let tmp_dir = TempDir::new().unwrap();
    let downloader = Downloader::with_policy(
        tmp_dir.path(),
        TransportPolicy::plaintext_for_tests().with_max_metadata_body(64),
    )
    .unwrap();

    server.mock(|when, then| {
        when.method(GET).path("/api/huge");
        then.status(200)
            .header("content-type", "application/json")
            .body(format!(r#"{{"status": "{}"}}"#, "x".repeat(4096)));
    });

    let result: Result<TestData, DownloadError> = downloader
        .download_json_no_cache(&server.url("/api/huge"))
        .await;

    assert!(matches!(
        result,
        Err(DownloadError::BodyTooLarge { limit: 64, .. })
    ));
}

/// The shipping policy refuses plaintext outright, so a downgraded catalog URL cannot silently
/// become the source of truth.
#[tokio::test]
async fn the_default_policy_refuses_plaintext_urls() {
    let server = MockServer::start();
    let tmp_dir = TempDir::new().unwrap();
    let downloader = Downloader::new(tmp_dir.path()).unwrap();

    let mock = server.mock(|when, then| {
        when.method(GET).path("/file.txt");
        then.status(200).body("should never be fetched");
    });

    let err = downloader
        .download(server.url("/file.txt"))
        .await
        .expect_err("http must be refused");

    assert!(matches!(err, DownloadError::InsecureUrl { .. }));
    mock.assert_calls(0);
}
