#![cfg(feature = "mock_sd")]


use std::io::{Cursor, Read};

use gem_flasher_sd::mock_sd::MockSd;
use gem_flasher_sd::{ContentType, Customization, Destination, ParitionType};

#[test]
fn flash_applies_customization_through_public_api() {
    let mut mock = MockSd::new();
    let image_bytes: Box<[u8]> = std::fs::read(mock.path()).unwrap().into_boxed_slice();
    let img_size = image_bytes.len() as u64;

    let img_resolver = move || Ok((Cursor::new(image_bytes), img_size));

    const FILE_NAME: &str = "customization.txt";
    const FILE_DATA: &[u8] = b"hello from the flasher test";
    let content = vec![(FILE_NAME.into(), FILE_DATA.to_vec().into_boxed_slice())]
        .into_iter()
        .map(|(name, data): (Box<str>, Box<[u8]>)| (name, ContentType::DataAppend(data)));
    let customization = Customization {
        partition: ParitionType::Boot,
        content,
    };

    gem_flasher_sd::flash(
        img_resolver,
        Destination::File(mock.path().into()),
        None,
        std::iter::once(customization),
        None,
    )
    .expect("flash with customization should succeed");

    let fs = mock.open_boot();
    let mut contents = String::new();
    fs.root_dir()
        .open_file(FILE_NAME)
        .expect("customization file should exist in boot partition")
        .read_to_string(&mut contents)
        .unwrap();
    assert_eq!(contents.as_bytes(), FILE_DATA);
}
