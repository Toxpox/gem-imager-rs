use super::*;

use std::io::{Cursor, Read, Write};
use tempfile::NamedTempFile;
#[cfg(feature = "piped_image")]
use tokio::io::AsyncWriteExt;
use zip::write::SimpleFileOptions;

#[test]
fn detects_uncompressed_image_and_reads_contents() {

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

    let file = tempfile::NamedTempFile::new().unwrap();

    let res = OsImage::from_path(file.path(), ExtractGate::LocalFile);
    assert!(res.is_err());
}

#[test]
fn rejects_truncated_xz_header() {

    let fake_xz = [0xfd, b'7', b'z', b'X', b'Z', 0x00, 0x01, 0x02, 0x03];

    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(&fake_xz).unwrap();
    file.flush().unwrap();

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
        }
    }
}

#[tokio::test]
#[cfg(feature = "piped_image")]
async fn file_stream_uncompressed_image_reads_contents() {

    let data = b"plain raw image data";

    let (mut writer, reader) = gem_helper::file_stream::file_stream().unwrap();

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

    let original = b"this is the uncompressed payload";
    let compressed = liblzma::encode_all(original.as_slice(), 6).unwrap();

    let (mut writer, reader) = gem_helper::file_stream::file_stream().unwrap();

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

    let (mut writer, reader) = gem_helper::file_stream::file_stream().unwrap();

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

    let result = OsImage::from_path(file.path(), gate).and_then(|mut img| {
        let mut out = Vec::new();
        img.read_to_end(&mut out)?;
        Ok(out)
    });

    assert!(result.is_err(), "a truncated xz stream must fail");
}

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

    let (mut writer, reader) = gem_helper::file_stream::file_stream().unwrap();
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
