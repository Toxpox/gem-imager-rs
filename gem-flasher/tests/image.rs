#![cfg(feature = "sd")]

//! Integration tests for the public `LocalImage` type.
//! `OsImage` compression detection is already covered by inline tests in
//! src/img/test.rs, but `LocalImage` had no coverage.

use std::io::Read;

use gem_flasher::LocalImage;

fn write_temp(dir: &std::path::Path, name: &str, data: &[u8]) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, data).unwrap();
    path
}

#[test]
fn local_image_accessors_and_display() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_temp(dir.path(), "myimage.img", b"raw image bytes");

    let img = LocalImage::new(path.clone().into_boxed_path());
    assert_eq!(img.path(), path.as_path());
    assert_eq!(img.file_name(), "myimage.img");
    // Display renders the file name.
    assert_eq!(img.to_string(), "myimage.img");
}

#[test]
fn local_image_into_image_fn_reads_uncompressed_contents() {
    let dir = tempfile::tempdir().unwrap();
    let data = b"raw uncompressed image payload";
    let path = write_temp(dir.path(), "os.img", data);

    let resolver = LocalImage::new(path.into_boxed_path()).into_image_fn();
    let (mut img, size) = resolver().unwrap();

    assert_eq!(size, data.len() as u64);
    let mut out = Vec::new();
    img.read_to_end(&mut out).unwrap();
    assert_eq!(out, data);
}
