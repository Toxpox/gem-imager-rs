
#[test]
fn drive_list_succeeds_and_descriptors_are_well_formed() {
    let devices = gem_drivelist::drive_list().expect("drive_list should succeed");

    for device in &devices {
        assert!(
            !device.enumerator.is_empty(),
            "every descriptor should have an enumerator: {device:?}"
        );
        assert!(
            !device.raw.is_empty(),
            "every descriptor should have a raw device id: {device:?}"
        );
    }
}
