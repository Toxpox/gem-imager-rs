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

fn test_downloader(cache: &Path) -> Downloader {
    Downloader::with_policy(cache, TransportPolicy::plaintext_for_tests()).unwrap()
}

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

#[tokio::test]
async fn download_archive_persists_verified_file_and_reports_progress() {
    let server = MockServer::start();
    let tmp = TempDir::new().unwrap();
    let downloader = test_downloader(tmp.path());

    let content = b"archive payload for download_archive";
    let sha = sha256(content);

    let mock = server.mock(|when, then| {
        when.method(GET).path("/a.img.xz");
        then.status(200).body(content);
    });

    use std::sync::{Arc, Mutex};
    let last_received = Arc::new(Mutex::new(0u64));
    let progress = last_received.clone();

    let path = downloader
        .download_archive(
            server.url("/a.img.xz"),
            ArchiveIntegrity::new(sha, content.len() as u64),
            move |received| {
                *progress.lock().unwrap() = received;
            },
        )
        .await
        .expect("matching sha should succeed");

    mock.assert_calls(1);
    assert_eq!(std::fs::read(&path).unwrap(), content);
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        const_hex::encode(sha)
    );
    assert_eq!(*last_received.lock().unwrap(), content.len() as u64);

    let again = downloader
        .download_archive(
            server.url("/a.img.xz"),
            ArchiveIntegrity::new(sha, content.len() as u64),
            |_| {},
        )
        .await
        .expect("cache hit should succeed");
    assert_eq!(again, path);
    mock.assert_calls(1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_download_archive_for_the_same_digest_fetches_once() {
    use std::sync::Arc;
    use std::time::Duration;

    let server = MockServer::start();
    let tmp = TempDir::new().unwrap();
    let downloader = Arc::new(test_downloader(tmp.path()));

    let content = b"archive payload shared by two concurrent callers";
    let sha = sha256(content);

    let mock = server.mock(|when, then| {
        when.method(GET).path("/shared.img.xz");
        then.status(200)
            .delay(Duration::from_millis(300))
            .body(content);
    });

    let integrity = ArchiveIntegrity::new(sha, content.len() as u64);

    let one = {
        let downloader = downloader.clone();
        let url = server.url("/shared.img.xz");
        tokio::spawn(async move { downloader.download_archive(url, integrity, |_| {}).await })
    };
    let two = {
        let downloader = downloader.clone();
        let url = server.url("/shared.img.xz");
        tokio::spawn(async move { downloader.download_archive(url, integrity, |_| {}).await })
    };

    let first = one.await.unwrap().expect("first download succeeds");
    let second = two.await.unwrap().expect("second download succeeds");

    assert_eq!(first, second);
    assert_eq!(std::fs::read(&first).unwrap(), content);
    mock.assert_calls(1);
}

#[tokio::test]
async fn download_archive_rejects_hash_mismatch_and_leaves_no_scratch() {
    let server = MockServer::start();
    let tmp = TempDir::new().unwrap();
    let downloader = test_downloader(tmp.path());

    let content = b"payload whose declared digest will be wrong";
    let wrong_sha = sha256(b"a different payload entirely");

    server.mock(|when, then| {
        when.method(GET).path("/bad.img.xz");
        then.status(200).body(content);
    });

    let result = downloader
        .download_archive(
            server.url("/bad.img.xz"),
            ArchiveIntegrity::new(wrong_sha, content.len() as u64),
            |_| {},
        )
        .await;

    match result {
        Err(DownloadError::ArchiveHashMismatch { .. }) => {}
        other => panic!("expected ArchiveHashMismatch, got {other:?}"),
    }

    for entry in entries(tmp.path()) {
        assert!(
            !entry
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains("part-"),
            "scratch file must not survive a failed download: {}",
            entry.display()
        );
    }
}

#[tokio::test]
async fn a_cancelled_archive_download_leaves_no_scratch_file() {
    // `download_archive` kept its scratch file alive deliberately so it could be renamed, which
    // meant an aborted download left a `.part-*` file of whatever had been received. For an OS
    // image that is gigabytes per cancelled attempt.
    let server = MockServer::start();
    let tmp = TempDir::new().unwrap();
    let downloader = test_downloader(tmp.path());

    let content = vec![3u8; 512 * 1024];
    let sha = sha256(&content);

    server.mock(|when, then| {
        when.method(GET).path("/slow.img.xz");
        then.status(200)
            .delay(std::time::Duration::from_secs(5))
            .body(content.clone());
    });

    let task = {
        let downloader = downloader.clone();
        let url = server.url("/slow.img.xz");
        let len = content.len() as u64;
        tokio::spawn(async move {
            downloader
                .download_archive(url, ArchiveIntegrity::new(sha, len), |_| {})
                .await
        })
    };

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    task.abort();
    let _ = task.await;

    // The abort unwinds the task, and the scratch guard's Drop runs on that unwind. Give the
    // runtime a moment to finish tearing the task down before inspecting the directory.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let leftovers: Vec<_> = entries(tmp.path())
        .into_iter()
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".part-")
        })
        .collect();
    assert!(
        leftovers.is_empty(),
        "an aborted archive download must leave no scratch file: {leftovers:?}"
    );
}

#[tokio::test]
async fn a_failed_publish_leaves_no_scratch_file() {
    // The rename that publishes a verified download can fail: a full disk, a permission change,
    // or on Windows another process holding the destination open. The scratch file was leaked in
    // exactly that case, because only a streaming error triggered cleanup.
    let server = MockServer::start();
    let tmp = TempDir::new().unwrap();
    let downloader = test_downloader(tmp.path());

    let content = b"payload whose publication will be blocked";
    let sha = sha256(content);

    server.mock(|when, then| {
        when.method(GET).path("/blocked.img.xz");
        then.status(200).body(content);
    });

    // Occupying the destination path with a directory makes `rename` fail on every platform
    // without needing to manipulate permissions.
    let destination = tmp.path().join(const_hex::encode(sha));
    std::fs::create_dir(&destination).unwrap();

    let result = downloader
        .download_archive(
            server.url("/blocked.img.xz"),
            ArchiveIntegrity::new(sha, content.len() as u64),
            |_| {},
        )
        .await;

    assert!(result.is_err(), "publishing into a directory must fail");

    let leftovers: Vec<_> = entries(tmp.path())
        .into_iter()
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".part-")
        })
        .collect();
    assert!(
        leftovers.is_empty(),
        "a failed publish must not leak the scratch file: {leftovers:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_separate_downloaders_sharing_a_cache_fetch_once() {
    // The single-flight map lives inside one Downloader. Two instances over the same cache
    // directory, which is what two CLI invocations or a GUI plus a helper amount to, each
    // started their own download of the same image.
    let server = MockServer::start();
    let tmp = TempDir::new().unwrap();

    let content = b"image bytes wanted by two independent downloaders";
    let sha = sha256(content);

    let mock = server.mock(|when, then| {
        when.method(GET).path("/shared-across-instances.img.xz");
        then.status(200)
            .delay(std::time::Duration::from_millis(300))
            .body(content);
    });

    let integrity = ArchiveIntegrity::new(sha, content.len() as u64);

    let one = {
        let downloader = test_downloader(tmp.path());
        let url = server.url("/shared-across-instances.img.xz");
        tokio::spawn(async move { downloader.download_archive(url, integrity, |_| {}).await })
    };
    let two = {
        let downloader = test_downloader(tmp.path());
        let url = server.url("/shared-across-instances.img.xz");
        tokio::spawn(async move { downloader.download_archive(url, integrity, |_| {}).await })
    };

    let first = one.await.unwrap().expect("first download succeeds");
    let second = two.await.unwrap().expect("second download succeeds");

    assert_eq!(first, second);
    assert_eq!(std::fs::read(&first).unwrap(), content);
    mock.assert_calls(1);
}

#[tokio::test]
async fn a_cache_hit_is_refused_when_the_declared_size_disagrees() {
    // A digest match settles what the bytes are, but a caller asking for a different size is
    // working from metadata that does not describe this file. Returning the cache entry anyway
    // hid that disagreement instead of surfacing it.
    let server = MockServer::start();
    let tmp = TempDir::new().unwrap();
    let downloader = test_downloader(tmp.path());

    let content = b"archive whose size will later be misdeclared";
    let sha = sha256(content);

    let mock = server.mock(|when, then| {
        when.method(GET).path("/sized.img.xz");
        then.status(200).body(content);
    });

    let path = downloader
        .download_archive(
            server.url("/sized.img.xz"),
            ArchiveIntegrity::new(sha, content.len() as u64),
            |_| {},
        )
        .await
        .expect("the first download succeeds");
    assert!(path.exists());
    mock.assert_calls(1);

    // Same digest, wrong declared size: the stale entry is discarded and refetched rather than
    // silently served.
    let result = downloader
        .download_archive(
            server.url("/sized.img.xz"),
            ArchiveIntegrity::new(sha, content.len() as u64 + 1),
            |_| {},
        )
        .await;

    match result {
        Err(DownloadError::ArchiveSizeMismatch {
            expected, actual, ..
        }) => {
            assert_eq!(expected, content.len() as u64 + 1);
            assert_eq!(actual, content.len() as u64);
        }
        other => panic!("expected ArchiveSizeMismatch, got {other:?}"),
    }
    mock.assert_calls(2);

    // The matching request still works afterwards.
    let again = downloader
        .download_archive(
            server.url("/sized.img.xz"),
            ArchiveIntegrity::new(sha, content.len() as u64),
            |_| {},
        )
        .await
        .expect("the correctly sized request still resolves");
    assert_eq!(std::fs::read(again).unwrap(), content);
}
