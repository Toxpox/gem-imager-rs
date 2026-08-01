use super::*;

use std::io::{Cursor, Read, Write};
use tempfile::NamedTempFile;
#[cfg(feature = "piped_image")]
use tokio::io::AsyncWriteExt;
use zip::write::SimpleFileOptions;

#[test]
fn detects_uncompressed_image_and_reads_contents() {
    // This is the most fundamental behavior test for OsImage:
    //
    // - verifies that non-compressed files fall back to the
    //   `Uncompressed` variant
    // - verifies that reported size matches actual file size
    // - verifies that `Read` delegation works correctly
    //
    // If this fails, even the simplest `.img` files would break.

    let data = b"plain raw image data";

    let mut file = NamedTempFile::new().unwrap();
    file.write_all(data).unwrap();
    file.flush().unwrap();

    let mut img = OsImage::from_path(file.path(), ExtractGate::LocalFile).unwrap();

    assert_eq!(img.size(), data.len() as u64);

    let mut out = Vec::new();
    img.read_to_end(&mut out).unwrap();

    assert_eq!(out, data);
}

#[test]
fn detects_xz_compressed_image_and_reports_uncompressed_size() {
    // This test validates the entire XZ detection path:
    //
    // - verifies that XZ magic bytes are detected correctly
    // - verifies that the decoder is constructed successfully
    // - verifies that `liblzma::uncompressed_size()` is used properly
    // - verifies that the internal rewind after probing works
    // - verifies that reads return decompressed contents instead of raw bytes
    //
    // The rewind verification is especially important here:
    // `OsImageCompression::new()` consumes bytes while probing magic,
    // and `from_path()` probes again to determine uncompressed size.
    // If either rewind is missing, reads would begin from the wrong offset
    // or decompression could fail entirely.

    let original = b"this is the uncompressed payload";

    let compressed = liblzma::encode_all(original.as_slice(), 6).unwrap();

    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(&compressed).unwrap();
    file.flush().unwrap();

    let mut img = OsImage::from_path(file.path(), ExtractGate::LocalFile).unwrap();

    assert_eq!(img.size(), original.len() as u64);

    let mut out = Vec::new();
    img.read_to_end(&mut out).unwrap();

    assert_eq!(out, original);
}

#[test]
fn detects_zip_compressed_image_and_reads_first_entry_contents() {
    // This test validates ZIP handling behavior:
    //
    // - verifies ZIP magic byte detection
    // - verifies that `stream_zip_entries_throwing_caution_to_the_wind()`
    //   successfully creates a streaming reader
    // - verifies that reported size comes from the ZIP entry metadata
    // - verifies that reads transparently decompress ZIP contents
    //
    // ZIP handling is structurally different from XZ:
    // unlike XZ, the uncompressed size is not probed from the stream itself,
    // but instead comes from ZIP metadata (`entry().uncompressed_size`).
    //
    // This test protects against:
    // - broken ZIP magic matching
    // - incorrect entry selection
    // - metadata regressions
    // - accidentally returning compressed bytes instead of decompressed data

    let original = b"zip payload contents";

    let mut zip_data = Cursor::new(Vec::<u8>::new());

    {
        let mut writer = zip::ZipWriter::new(&mut zip_data);

        writer
            .start_file("image.img", SimpleFileOptions::default())
            .unwrap();

        writer.write_all(original).unwrap();

        writer.finish().unwrap();
    }

    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(zip_data.get_ref()).unwrap();
    file.flush().unwrap();

    let mut img = OsImage::from_path(file.path(), ExtractGate::LocalFile).unwrap();

    assert_eq!(img.size(), original.len() as u64);

    let mut out = Vec::new();
    img.read_to_end(&mut out).unwrap();

    assert_eq!(out, original);
}

#[test]
fn rejects_empty_file_during_format_detection() {
    // This test verifies behavior for completely empty inputs.
    //
    // `OsImageCompression::new()` always attempts to read 6 bytes
    // for magic detection. An empty file therefore cannot possibly
    // contain a valid header and must fail immediately.
    //
    // This protects against:
    // - accidentally treating empty files as valid raw images
    // - silent short-read bugs during magic probing
    // - regressions where partial header reads become ignored
    //
    // The exact error kind is intentionally *not* asserted here,
    // because different readers/platforms may surface slightly
    // different IO errors (`UnexpectedEof` is the common case).

    let file = tempfile::NamedTempFile::new().unwrap();

    let res = OsImage::from_path(file.path(), ExtractGate::LocalFile);
    assert!(res.is_err());
}

#[test]
fn rejects_truncated_xz_header() {
    // This test verifies behavior for files that *look* like XZ
    // based on magic bytes, but do not contain a valid XZ stream.
    //
    // This is important because format detection is optimistic:
    // once the magic matches, the code immediately constructs an
    // XZ decoder.
    //
    // Without this test, a regression could accidentally:
    // - accept corrupt/truncated compressed images
    // - delay failures until much later during flashing
    // - report incorrect image sizes
    //
    // The test intentionally provides:
    // - a correct XZ magic header
    // - but no valid XZ payload afterwards

    // Valid XZ magic bytes followed by garbage/truncated payload.
    let fake_xz = [0xfd, b'7', b'z', b'X', b'Z', 0x00, 0x01, 0x02, 0x03];

    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(&fake_xz).unwrap();
    file.flush().unwrap();

    // Construction itself may succeed because decoding is lazy.
    // The important part is that actual usage must fail.
    let result = OsImage::from_path(file.path(), ExtractGate::LocalFile);

    match result {
        Ok(mut img) => {
            let mut out = Vec::new();

            assert!(
                img.read_to_end(&mut out).is_err(),
                "truncated XZ stream unexpectedly succeeded"
            );
        }
        Err(_) => {
            // Also acceptable:
            // some decoder versions/platforms fail eagerly.
        }
    }
}

#[tokio::test]
#[cfg(feature = "piped_image")]
async fn file_stream_uncompressed_image_reads_contents() {
    // This test validates that `OsImage::from_piped()` behaves the same
    // as `from_path()` for plain uncompressed images.
    //
    // This is important because the piped/streamed code path uses a
    // completely different backing source (`FileStream`) while still
    // relying on the exact same compression detection logic.
    //
    // The test protects against:
    // - stream rewind bugs during magic probing
    // - FileStream-specific read issues
    // - accidental divergence between path and stream behavior
    // - regressions in uncompressed fallback handling

    let data = b"plain raw image data";

    let (mut writer, reader) = bb_helper::file_stream::file_stream().unwrap();

    writer.write_all(data).await.unwrap();
    writer.flush().await.unwrap();
    drop(writer);

    tokio::task::spawn_blocking(move || {
        let abort = tokio::spawn(async { Ok(()) });
        let mut img = OsImage::from_piped(
            reader,
            AbortOnDropHandle::new(abort),
            data.len() as u64,
            ExtractGate::LocalFile,
        )
        .unwrap();

        assert_eq!(img.size(), data.len() as u64);

        let mut out = Vec::new();
        img.read_to_end(&mut out).unwrap();

        assert_eq!(out, data);
    })
    .await
    .unwrap()
}

#[tokio::test]
#[cfg(feature = "piped_image")]
async fn file_stream_xz_image_reports_uncompressed_size_and_reads_contents() {
    // This test validates streamed XZ handling.
    //
    // XZ is particularly important here because detection requires:
    // - reading magic bytes
    // - rewinding the stream afterwards
    // - performing decompression lazily during reads
    //
    // Stream-backed readers are historically more fragile than regular
    // files because seek/replay semantics are often emulated.
    //
    // This protects against:
    // - broken rewind support in FileStream
    // - decoder initialization issues
    // - partial/deferred decompression failures
    // - mismatched streamed vs file-backed behavior

    let original = b"this is the uncompressed payload";
    let compressed = liblzma::encode_all(original.as_slice(), 6).unwrap();

    let (mut writer, reader) = bb_helper::file_stream::file_stream().unwrap();

    writer.write_all(&compressed).await.unwrap();
    writer.flush().await.unwrap();
    drop(writer);

    tokio::task::spawn_blocking(move || {
        let abort = tokio::spawn(async { Ok(()) });
        let mut img = OsImage::from_piped(
            reader,
            AbortOnDropHandle::new(abort),
            original.len() as u64,
            ExtractGate::LocalFile,
        )
        .unwrap();

        assert_eq!(img.size(), original.len() as u64);

        let mut out = Vec::new();
        img.read_to_end(&mut out).unwrap();

        assert_eq!(out, original);
    })
    .await
    .unwrap()
}

#[tokio::test]
#[cfg(feature = "piped_image")]
async fn file_stream_zip_image_reads_first_entry_contents() {
    // This test validates ZIP handling over FileStream-backed input.
    //
    // ZIP streaming is especially valuable to test because ZIP readers
    // often assume normal filesystem semantics internally.
    //
    // This protects against:
    // - incompatibilities between ZIP streaming and FileStream
    // - incorrect incremental reads
    // - broken ZIP entry parsing over streamed input
    // - regressions where only file-backed ZIPs work correctly

    let original = b"zip payload contents";

    let mut zip_data = Cursor::new(Vec::<u8>::new());
    {
        let mut writer = zip::ZipWriter::new(&mut zip_data);
        writer
            .start_file("image.img", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(original).unwrap();
        writer.finish().unwrap();
    }

    let (mut writer, reader) = bb_helper::file_stream::file_stream().unwrap();

    writer.write_all(zip_data.get_ref()).await.unwrap();
    writer.flush().await.unwrap();
    drop(writer);

    tokio::task::spawn_blocking(move || {
        let abort = tokio::spawn(async { Ok(()) });
        let mut img = OsImage::from_piped(
            reader,
            AbortOnDropHandle::new(abort),
            original.len() as u64,
            ExtractGate::LocalFile,
        )
        .unwrap();

        assert_eq!(img.size(), original.len() as u64);

        let mut out = Vec::new();
        img.read_to_end(&mut out).unwrap();

        assert_eq!(out, original);
    })
    .await
    .unwrap()
}

// ---------------------------------------------------------------------------
// Extracted-side integrity gates (the integrity policy, test matrix the integrity test matrix)
//
// These drive the gate through the real decoder rather than the verifier's unit tests, so they
// also prove the gate is wired into `Read` for every code path the front-ends use.
// ---------------------------------------------------------------------------

fn sha256_of(data: &[u8]) -> [u8; 32] {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

fn xz_file(payload: &[u8]) -> NamedTempFile {
    let compressed = liblzma::encode_all(payload, 6).unwrap();
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(&compressed).unwrap();
    file.flush().unwrap();
    file
}

#[test]
fn a_declared_gate_passes_when_the_extracted_bytes_match() {
    let payload = b"the payload the catalog published";
    let file = xz_file(payload);

    let gate = ExtractGate::Declared(ExtractedIntegrity::new(
        payload.len() as u64,
        sha256_of(payload),
    ));
    let mut img = OsImage::from_path(file.path(), gate).unwrap();

    let mut out = Vec::new();
    img.read_to_end(&mut out).unwrap();
    assert_eq!(out, payload);
}

/// The archive can be exactly what the catalog published while the *extracted* digest is wrong —
/// a rebuilt image republished under an unchanged entry, for instance. Only this gate catches it.
#[test]
fn a_declared_gate_fails_when_the_extracted_sha256_differs() {
    let payload = b"the payload the catalog published";
    let file = xz_file(payload);

    let gate = ExtractGate::Declared(ExtractedIntegrity::new(
        payload.len() as u64,
        sha256_of(b"something else entirely, same length!"),
    ));
    let mut img = OsImage::from_path(file.path(), gate).unwrap();

    let mut out = Vec::new();
    let err = img
        .read_to_end(&mut out)
        .expect_err("a wrong extracted digest must fail the read");

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("sha256"), "{err}");
}

#[test]
fn a_declared_gate_fails_when_the_extracted_stream_is_shorter_than_declared() {
    let payload = b"short payload";
    let file = xz_file(payload);

    let gate = ExtractGate::Declared(ExtractedIntegrity::new(
        payload.len() as u64 + 4096,
        sha256_of(payload),
    ));
    let mut img = OsImage::from_path(file.path(), gate).unwrap();

    let mut out = Vec::new();
    let err = img
        .read_to_end(&mut out)
        .expect_err("a stream shorter than the manifest must fail");

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn a_declared_gate_fails_when_the_extracted_stream_is_longer_than_declared() {
    let payload = vec![0xABu8; 64 * 1024];
    let file = xz_file(&payload);

    let gate = ExtractGate::Declared(ExtractedIntegrity::new(1024, sha256_of(&payload)));
    let mut img = OsImage::from_path(file.path(), gate).unwrap();

    let mut out = Vec::new();
    let err = img
        .read_to_end(&mut out)
        .expect_err("a stream longer than the manifest must fail");

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert!(
        out.len() < payload.len(),
        "the read must stop as soon as the size gate is provably broken"
    );
}

/// A truncated archive must be an error, never a short-but-successful write. The decoder itself
/// reports the truncation; the point of the test is that nothing downgrades it to a warning.
#[test]
fn a_truncated_xz_archive_fails_to_decode() {
    let payload = vec![0x5Au8; 256 * 1024];
    let compressed = liblzma::encode_all(payload.as_slice(), 6).unwrap();

    let mut file = NamedTempFile::new().unwrap();
    file.write_all(&compressed[..compressed.len() / 2]).unwrap();
    file.flush().unwrap();

    let gate = ExtractGate::Declared(ExtractedIntegrity::new(
        payload.len() as u64,
        sha256_of(&payload),
    ));

    // The truncation is caught while probing the xz footer, before a single byte is written.
    // Either failure point is acceptable; silently writing a short image is not.
    let result = OsImage::from_path(file.path(), gate).and_then(|mut img| {
        let mut out = Vec::new();
        img.read_to_end(&mut out)?;
        Ok(out)
    });

    assert!(result.is_err(), "a truncated xz stream must fail");
}

/// Trailing bytes after the xz stream are refused rather than silently ignored: an image whose
/// tail nobody accounts for is an image nobody can vouch for.
#[test]
fn trailing_garbage_after_an_xz_stream_is_refused() {
    let payload = b"a well formed payload";
    let mut compressed = liblzma::encode_all(payload.as_slice(), 6).unwrap();
    compressed.extend_from_slice(b"trailing garbage that belongs to nothing");

    let mut file = NamedTempFile::new().unwrap();
    file.write_all(&compressed).unwrap();
    file.flush().unwrap();

    let gate = ExtractGate::Declared(ExtractedIntegrity::new(
        payload.len() as u64,
        sha256_of(payload),
    ));

    // Rejected while probing the xz footer, which the garbage displaced.
    let result = OsImage::from_path(file.path(), gate).and_then(|mut img| {
        let mut out = Vec::new();
        img.read_to_end(&mut out)?;
        Ok(out)
    });

    assert!(
        result.is_err(),
        "trailing garbage must not be accepted silently"
    );
}

#[cfg(feature = "piped_image")]
#[tokio::test]
async fn the_gate_also_applies_to_a_piped_download() {
    let payload = b"streamed straight from the download";
    let compressed = liblzma::encode_all(payload.as_slice(), 6).unwrap();

    let (mut writer, reader) = bb_helper::file_stream::file_stream().unwrap();
    writer.write_all(&compressed).await.unwrap();
    writer.flush().await.unwrap();
    drop(writer);

    tokio::task::spawn_blocking(move || {
        let abort = tokio::spawn(async { Ok(()) });
        let gate = ExtractGate::Declared(ExtractedIntegrity::new(
            payload.len() as u64,
            sha256_of(b"a different payload of the same size!"),
        ));
        let mut img = OsImage::from_piped(
            reader,
            AbortOnDropHandle::new(abort),
            payload.len() as u64,
            gate,
        )
        .unwrap();

        let mut out = Vec::new();
        let err = img
            .read_to_end(&mut out)
            .expect_err("the gate must apply to the piped path too");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    })
    .await
    .unwrap()
}
