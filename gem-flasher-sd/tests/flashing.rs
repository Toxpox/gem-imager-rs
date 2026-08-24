use std::{
    io::{Cursor, Read, Seek},
    sync::mpsc,
};

use gem_flasher_sd::{ContentType, Customization, Destination, Status};
use tempfile::NamedTempFile;

fn test_file(len: usize) -> std::io::Cursor<Box<[u8]>> {
    let data: Vec<u8> = (0..len)
        .map(|x| x % 255)
        .map(|x| u8::try_from(x).unwrap())
        .collect();
    std::io::Cursor::new(data.into())
}

type NoCustomizations =
    std::iter::Empty<Customization<std::iter::Empty<(Box<str>, ContentType<'static>)>>>;

fn no_customizations() -> NoCustomizations {
    std::iter::empty()
}

#[test]
fn test_public_flash_with_temp_file() {
    const FILE_LEN: usize = 16 * 1024;
    let dummy_file = test_file(FILE_LEN);
    let expected_bytes = dummy_file.get_ref().clone();

    let temp_destination = NamedTempFile::new().expect("Failed to create temp file");
    let dst = Destination::File(temp_destination.path().into());

    let img_data = expected_bytes.clone();
    let img_resolver = move || {
        let reader = Cursor::new(img_data);
        Ok((reader, FILE_LEN as u64))
    };

    let (tx, rx) = mpsc::sync_channel(32);

    let result = gem_flasher_sd::flash(img_resolver, dst, Some(tx), no_customizations(), None);

    assert!(result.is_ok(), "Public flash failed: {:?}", result.err());

    let mut written_file = temp_destination
        .reopen()
        .expect("Failed to reopen temp file");
    let mut written_bytes = Vec::new();

    written_file.rewind().unwrap();
    written_file.read_to_end(&mut written_bytes).unwrap();

    assert_eq!(written_bytes.len(), FILE_LEN);
    assert_eq!(written_bytes, expected_bytes.into_vec());

    let updates: Vec<Status> = rx.try_iter().collect();
    assert!(
        updates
            .iter()
            .any(|s| matches!(s, Status::Writing(x) if (*x - 1.0).abs() < f32::EPSILON)),
        "write progress never reached 1.0: {updates:?}"
    );
    assert!(
        updates.iter().any(|s| matches!(s, Status::Verifying(_))),
        "no read-back verification stage was reported: {updates:?}"
    );
}

#[test]
fn stages_are_reported_in_pipeline_order() {
    const FILE_LEN: usize = 16 * 1024;
    let img_data = test_file(FILE_LEN).get_ref().clone();

    let temp_destination = NamedTempFile::new().unwrap();
    let (tx, rx) = mpsc::sync_channel(256);

    gem_flasher_sd::flash(
        move || Ok((Cursor::new(img_data), FILE_LEN as u64)),
        Destination::File(temp_destination.path().into()),
        Some(tx),
        no_customizations(),
        None,
    )
    .unwrap();

    const fn rank(s: &Status) -> u8 {
        match s {
            Status::Preparing => 0,
            Status::Writing(_) => 1,
            Status::Verifying(_) => 2,
            Status::Customizing => 3,
        }
    }

    let updates: Vec<Status> = rx.try_iter().collect();
    let mut last = 0;
    for update in &updates {
        let r = rank(update);
        assert!(
            r >= last,
            "stage went backwards at {update:?} in {updates:?}"
        );
        last = r;
    }
}

#[test]
fn flash_aborts_with_cancelled_token() {
    use gem_helper::cancel::CancellationToken;

    const FILE_LEN: usize = 16 * 1024;
    let dummy_file = test_file(FILE_LEN);
    let img_data = dummy_file.get_ref().clone();

    let temp_destination = NamedTempFile::new().expect("Failed to create temp file");
    let dst = Destination::File(temp_destination.path().into());

    let img_resolver = move || Ok((Cursor::new(img_data), FILE_LEN as u64));

    let token = CancellationToken::default();
    drop(token.drop_guard());
    assert!(token.is_cancelled());

    let result = gem_flasher_sd::flash(img_resolver, dst, None, no_customizations(), Some(token));

    assert!(
        matches!(result, Err(gem_flasher_sd::Error::Aborted)),
        "expected Err(Aborted) for a pre-cancelled token, got {result:?}"
    );
}

#[test]
fn a_truncated_image_fails_instead_of_succeeding() {
    const DECLARED_LEN: u64 = 32 * 1024;
    const ACTUAL_LEN: usize = 8 * 1024;

    let img_data = test_file(ACTUAL_LEN).get_ref().clone();
    let temp_destination = NamedTempFile::new().unwrap();

    let result = gem_flasher_sd::flash(
        move || Ok((Cursor::new(img_data), DECLARED_LEN)),
        Destination::File(temp_destination.path().into()),
        None,
        no_customizations(),
        None,
    );

    assert!(
        matches!(result, Err(gem_flasher_sd::Error::ShortWrite { .. })),
        "expected ShortWrite, got {result:?}"
    );
}

#[test]
fn destinations() {
    let temp = gem_flasher_sd::devices(false);
    assert!(!temp.is_empty());
}
