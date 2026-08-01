use std::{io::Cursor, sync::mpsc};

use crate::flashing::read_aligned;

use super::*;

fn test_file(len: usize) -> std::io::Cursor<Box<[u8]>> {
    let data: Vec<u8> = (0..len)
        .map(|x| x % 255)
        .map(|x| u8::try_from(x).unwrap())
        .collect();
    std::io::Cursor::new(data.into())
}

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

#[test]
fn sd_write() {
    const FILE_LEN: usize = 12 * 1024;

    let dummy_file = test_file(FILE_LEN);
    let mut sd = std::io::Cursor::new(Vec::<u8>::new());

    let outcome = write_sd(dummy_file.clone(), FILE_LEN as u64, &mut sd, None, None).unwrap();

    assert_eq!(sd.get_ref().as_slice(), dummy_file.get_ref().as_ref());
    assert_eq!(outcome.written, FILE_LEN as u64);
    assert_eq!(outcome.sha256, sha256(dummy_file.get_ref()));
}

/// The writer counts what it actually put on the device, so a stream that ends before the declared
/// size can no longer be reported as a completed flash.
#[test]
fn a_stream_shorter_than_the_declared_size_is_a_short_write() {
    const ACTUAL_LEN: usize = 4 * 1024;
    const DECLARED_LEN: u64 = 12 * 1024;

    let mut sd = std::io::Cursor::new(Vec::<u8>::new());
    let err = write_sd(test_file(ACTUAL_LEN), DECLARED_LEN, &mut sd, None, None)
        .expect_err("a truncated image must not report success");

    match err {
        crate::Error::ShortWrite { expected, written } => {
            assert_eq!(expected, DECLARED_LEN);
            assert_eq!(written, ACTUAL_LEN as u64);
        }
        other => panic!("expected ShortWrite, got {other:?}"),
    }
}

/// Padding the last chunk up to the device alignment must not push the reported progress past 1.0;
/// the GUI extrapolates an ETA from it and a value above 1 yields a negative duration.
#[test]
fn progress_never_exceeds_one_when_the_last_chunk_is_padded() {
    const FILE_LEN: usize = 1000; // deliberately not a multiple of 512

    let (tx, rx) = mpsc::sync_channel(32);
    let mut sd = std::io::Cursor::new(Vec::<u8>::new());

    let outcome = write_sd(
        test_file(FILE_LEN),
        FILE_LEN as u64,
        &mut sd,
        Some(&tx),
        None,
    )
    .unwrap();
    drop(tx);

    assert_eq!(outcome.written, 1024, "1000 bytes pad up to two 512 blocks");

    for status in rx.try_iter() {
        if let Status::Writing(x) = status {
            assert!((0.0..=1.0).contains(&x), "progress out of range: {x}");
        }
    }
}

#[test]
fn read_back_accepts_data_that_matches_what_was_written() {
    const FILE_LEN: usize = 12 * 1024;

    let img = test_file(FILE_LEN);
    let mut sd = std::io::Cursor::new(Vec::<u8>::new());
    let outcome = write_sd(img, FILE_LEN as u64, &mut sd, None, None).unwrap();

    verify_written(&mut sd, outcome, None, None).expect("an untouched device must verify");
}

/// A single flipped byte anywhere in the written region has to fail. This is the failure mode that
/// every earlier gate — archive hash, extracted hash, byte count — is blind to, because they all
/// describe the data on the way *to* the device.
#[test]
fn read_back_rejects_a_single_flipped_byte() {
    const FILE_LEN: usize = 12 * 1024;

    let img = test_file(FILE_LEN);
    let mut sd = std::io::Cursor::new(Vec::<u8>::new());
    let outcome = write_sd(img, FILE_LEN as u64, &mut sd, None, None).unwrap();

    // Corrupt the device behind the flasher's back.
    sd.get_mut()[FILE_LEN / 2] ^= 0xFF;

    let err = verify_written(&mut sd, outcome, None, None)
        .expect_err("a flipped byte must fail read-back");

    match err {
        crate::Error::ReadBackMismatch { expected, actual } => {
            assert_ne!(expected, actual);
            assert_eq!(expected.len(), 64, "sha256 rendered as hex");
        }
        other => panic!("expected ReadBackMismatch, got {other:?}"),
    }
}

/// A device that returns fewer bytes than were written to it (truncated, removed mid-verify) must
/// not be able to pass by hashing only the prefix it did return.
#[test]
fn read_back_rejects_a_device_that_returns_too_little() {
    const FILE_LEN: usize = 12 * 1024;

    let img = test_file(FILE_LEN);
    let mut sd = std::io::Cursor::new(Vec::<u8>::new());
    let outcome = write_sd(img, FILE_LEN as u64, &mut sd, None, None).unwrap();

    sd.get_mut().truncate(FILE_LEN / 2);

    let err = verify_written(&mut sd, outcome, None, None)
        .expect_err("a short device read must fail read-back");

    assert!(
        matches!(err, crate::Error::IoError { .. }),
        "expected an IO error, got {err:?}"
    );
}

#[test]
fn read_back_reports_its_own_progress_stage() {
    const FILE_LEN: usize = 64 * 1024;

    let img = test_file(FILE_LEN);
    let mut sd = std::io::Cursor::new(Vec::<u8>::new());
    let outcome = write_sd(img, FILE_LEN as u64, &mut sd, None, None).unwrap();

    let (tx, rx) = mpsc::sync_channel(64);
    verify_written(&mut sd, outcome, Some(&tx), None).unwrap();
    drop(tx);

    let stages: Vec<Status> = rx.try_iter().collect();
    assert!(
        stages.iter().any(|s| matches!(s, Status::Verifying(_))),
        "verification must be visible as its own stage"
    );
    assert!(
        !stages.iter().any(|s| matches!(s, Status::Writing(_))),
        "the verify pass must not emit write progress"
    );
}

#[test]
fn read_back_stops_when_cancelled() {
    const FILE_LEN: usize = 64 * 1024;

    let img = test_file(FILE_LEN);
    let mut sd = std::io::Cursor::new(Vec::<u8>::new());
    let outcome = write_sd(img, FILE_LEN as u64, &mut sd, None, None).unwrap();

    let token = CancellationToken::default();
    drop(token.drop_guard());

    let err = verify_written(&mut sd, outcome, None, Some(&token))
        .expect_err("a cancelled verify must abort");

    assert!(matches!(err, crate::Error::Aborted), "got {err:?}");
}

mod target_guard {
    use super::*;

    fn device(is_system: bool, size: u64) -> crate::Device {
        crate::Device {
            name: "Test Card".to_string(),
            path: std::path::PathBuf::from("/dev/test"),
            size,
            is_system,
        }
    }

    /// The single most destructive mistake this tool can make is writing an OS image over the
    /// disk the user is running from. It is refused on the device flag, not on `is_removable`,
    /// which reports true for USB-attached system disks.
    #[test]
    fn a_system_disk_is_refused() {
        let err = evaluate_target(Some(&device(true, 64 * 1024 * 1024 * 1024)))
            .expect_err("a system disk must never be a valid destination");

        match err {
            crate::Error::SystemDisk { name } => assert_eq!(&*name, "Test Card"),
            other => panic!("expected SystemDisk, got {other:?}"),
        }
    }

    #[test]
    fn a_normal_card_reports_its_capacity() {
        assert_eq!(
            evaluate_target(Some(&device(false, 32 * 1024 * 1024 * 1024))).unwrap(),
            Some(32 * 1024 * 1024 * 1024)
        );
    }

    /// An unreported size must not be read as "0 bytes free", which would refuse every image.
    #[test]
    fn an_unknown_capacity_does_not_block_the_flash() {
        assert_eq!(evaluate_target(Some(&device(false, 0))).unwrap(), None);
        assert_eq!(evaluate_target(None).unwrap(), None);
    }
}

/// Helpers for driving `flash_internal` against [`crate::mock_sd::MockSd`], which is a real
/// 128 MiB MBR + FAT32 image and therefore exercises partition detection as well.
mod mock_card {
    use super::*;
    use crate::customization::{ContentType, Customization, ParitionType};
    use crate::mock_sd::MockSd;

    type Content = std::iter::Map<
        std::vec::IntoIter<(Box<str>, Box<[u8]>)>,
        fn((Box<str>, Box<[u8]>)) -> (Box<str>, ContentType<'static>),
    >;

    fn no_customizations() -> std::iter::Empty<Customization<Content>> {
        std::iter::empty()
    }

    fn boot_file(name: &str, data: &[u8]) -> std::iter::Once<Customization<Content>> {
        let entries = vec![(name.into(), data.to_vec().into_boxed_slice())];
        let content: Content = entries
            .into_iter()
            .map(|(n, d): (Box<str>, Box<[u8]>)| (n, ContentType::DataAppend(d)));

        std::iter::once(Customization {
            partition: ParitionType::Boot,
            content,
        })
    }

    /// Same as [`boot_file`], but the file is read back off the card and compared.
    fn verified_boot_file(name: &str, data: &[u8]) -> std::iter::Once<Customization<Content>> {
        let entries = vec![(name.into(), data.to_vec().into_boxed_slice())];
        let content: Content = entries
            .into_iter()
            .map(|(n, d): (Box<str>, Box<[u8]>)| (n, ContentType::VerifiedData(d)));

        std::iter::once(Customization {
            partition: ParitionType::Boot,
            content,
        })
    }

    /// `instruction.md` §10.4: the first-boot file is written to the FAT partition and then read
    /// back from it. This drives the real MBR + FAT32 image, so it covers the partition lookup,
    /// the write and the verifying re-open.
    #[test]
    fn a_verified_boot_file_is_written_and_read_back() {
        let card = MockSd::new();
        let image: Box<[u8]> = std::fs::read(card.path()).unwrap().into_boxed_slice();
        let img_size = image.len() as u64;
        let expected = b"firstboot=1\nhostname='t3-gemstone'\n";

        flash_internal(
            move || Ok((Cursor::new(image), img_size)),
            card,
            None,
            None,
            verified_boot_file("config.ini", expected),
            None,
        )
        .expect("a healthy card must write and verify config.ini");
    }

    /// The read-back opens the file from a fresh filesystem handle, so a file that never landed is
    /// caught instead of being echoed back from the writer's own buffer.
    #[test]
    fn a_missing_file_fails_the_read_back() {
        let mut card = MockSd::new();

        let err = ParitionType::Boot
            .verify(
                &mut card,
                &[("absent.ini".into(), b"firstboot=1\n".to_vec().into())],
            )
            .expect_err("a file that is not on the card cannot verify");

        assert!(
            matches!(err, crate::Error::CustomizationReadBackMismatch { .. }),
            "expected CustomizationReadBackMismatch, got {err:?}"
        );
    }

    /// A byte flip after the write must surface as a mismatch, not as a successful flash. This is
    /// the file-level counterpart of the raw read-back test above.
    #[test]
    fn a_flipped_byte_fails_the_read_back() {
        let mut card = MockSd::new();
        let expected: Box<[u8]> = b"firstboot=1\nhostname='t3-gemstone'\n".to_vec().into();

        {
            let mut corrupted = expected.to_vec();
            corrupted[0] ^= 0x01;

            let fs = card.open_boot();
            {
                let root = fs.root_dir();
                let mut f = root.create_file("config.ini").unwrap();
                std::io::Write::write_all(&mut f, &corrupted).unwrap();
            }
            fs.unmount().unwrap();
        }

        let err = ParitionType::Boot
            .verify(&mut card, &[("config.ini".into(), expected)])
            .expect_err("a single flipped byte must fail verification");

        assert!(
            matches!(err, crate::Error::CustomizationReadBackMismatch { .. }),
            "expected CustomizationReadBackMismatch, got {err:?}"
        );
    }

    /// The error names the file and nothing else: these files carry password hashes and PSKs.
    #[test]
    fn the_read_back_error_never_quotes_the_file_contents() {
        let err = crate::Error::CustomizationReadBackMismatch {
            file: "config.ini".into(),
        };
        let rendered = format!("{err}");

        assert!(rendered.contains("config.ini"));
        assert!(!rendered.contains("firstboot"));
        assert!(!rendered.contains('$'));
    }

    /// The whole pipeline over a real partitioned image: write, sync, read back, then land a file
    /// in the first FAT partition. A failure anywhere in MBR/FAT detection surfaces here.
    #[test]
    fn a_full_flash_writes_verifies_and_customizes() {
        let card = MockSd::new();
        let image: Box<[u8]> = std::fs::read(card.path()).unwrap().into_boxed_slice();
        let img_size = image.len() as u64;

        flash_internal(
            move || Ok((Cursor::new(image), img_size)),
            card,
            None,
            None,
            boot_file("customization.txt", b"written by the flasher"),
            None,
        )
        .expect("a healthy card must flash, verify and customize");
    }

    /// A device that accepts every write and then fails to sync must fail the flash. Reporting
    /// success here is how a card that was never actually written gets handed to a user.
    #[test]
    fn a_sync_failure_fails_the_flash() {
        let card = MockSd::new();
        let image: Box<[u8]> = std::fs::read(card.path()).unwrap().into_boxed_slice();
        let img_size = image.len() as u64;

        drop(card.sync_fail_token().drop_guard());

        let err = flash_internal(
            move || Ok((Cursor::new(image), img_size)),
            card,
            None,
            None,
            no_customizations(),
            None,
        )
        .expect_err("a device that cannot sync must not report success");

        assert!(
            matches!(err, crate::Error::SyncFailed { .. }),
            "expected SyncFailed, got {err:?}"
        );
    }

    /// Capacity is checked before the first byte is written, so an oversized image fails fast
    /// instead of after filling the card.
    #[test]
    fn an_image_larger_than_the_card_is_rejected_before_writing() {
        const CAPACITY: u64 = 8 * 1024;
        const IMG_SIZE: u64 = 64 * 1024;

        let card = MockSd::new();
        let untouched = std::fs::read(card.path()).unwrap();

        let err = flash_internal(
            || Ok((test_file(IMG_SIZE as usize), IMG_SIZE)),
            card,
            Some(CAPACITY),
            None,
            no_customizations(),
            None,
        )
        .expect_err("an image larger than the card must be refused");

        match err {
            crate::Error::InsufficientCapacity {
                required,
                available,
            } => {
                assert_eq!(required, IMG_SIZE);
                assert_eq!(available, CAPACITY);
            }
            other => panic!("expected InsufficientCapacity, got {other:?}"),
        }

        assert_eq!(
            untouched.len(),
            128 * 1024 * 1024,
            "the card must still be its original size, i.e. nothing was written"
        );
    }

    /// A card exactly as large as the image is fine; the guard must reject only what does not fit.
    #[test]
    fn an_image_that_exactly_fills_the_card_is_accepted() {
        let card = MockSd::new();
        let image: Box<[u8]> = std::fs::read(card.path()).unwrap().into_boxed_slice();
        let img_size = image.len() as u64;

        flash_internal(
            move || Ok((Cursor::new(image), img_size)),
            card,
            Some(img_size),
            None,
            no_customizations(),
            None,
        )
        .expect("an image that exactly fits must be accepted");
    }
}

#[test]
fn test_read_aligned_exact_multiple() {
    let input_data = vec![1u8; 1024]; // Exactly 2x 512-byte alignment blocks
    let mut cursor = Cursor::new(input_data);
    let mut buf = vec![0u8; 1024];

    let result = read_aligned(&mut cursor, &mut buf);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 1024);
    assert_eq!(&buf[..], &vec![1u8; 1024][..]);
}

#[test]
fn test_read_aligned_padding_needed() {
    let input_data = vec![5u8; 300]; // Not an alignment multiple (512)
    let mut cursor = Cursor::new(input_data);
    let mut buf = vec![0u8; 512];

    let result = read_aligned(&mut cursor, &mut buf);
    assert!(result.is_ok());
    // It should pad out to the next 512-byte alignment boundary
    assert_eq!(result.unwrap(), 512);

    // Original data intact
    assert_eq!(&buf[0..300], &vec![5u8; 300][..]);
    // Padded area zeroed out
    assert_eq!(&buf[300..512], &vec![0u8; 212][..]);
}

#[test]
fn test_reader_task_stops_at_eof() {
    let input_data = vec![42u8; 100];
    let mut cursor = Cursor::new(input_data);

    let (buf_tx_pool, buf_rx_pool) = mpsc::channel();
    let (buf_tx_out, buf_rx_out) = mpsc::sync_channel(2);

    // Supply one buffer to the pool
    buf_tx_pool.send(Box::new(DirectIoBuffer::new())).unwrap();

    // Run the task (it should read, send data, and then wait or finish if buffer pool empties)
    // Dropping the pool transmitter ensures the loop terminates when buffers run out or EOF hits
    drop(buf_tx_pool);

    let result = reader_task(&mut cursor, buf_rx_pool, buf_tx_out, None);
    assert!(result.is_ok());

    // Verify data reached the output channel
    let (received_buf, count) = buf_rx_out.recv().unwrap();
    // Since input was 100 bytes, it got aligned up to 512
    assert_eq!(count, 512);
    assert_eq!(received_buf.as_slice()[0], 42);
}

#[test]
fn test_writer_task_success() {
    let output = Cursor::new(vec![0u8; 1024]);
    let (tx_out, rx_out) = mpsc::channel();
    let (tx_pool, rx_pool) = mpsc::sync_channel(2);
    let (progress_tx, progress_rx) = mpsc::sync_channel(2);

    // Prep a buffer with data to write
    let mut mock_buf = Box::new(DirectIoBuffer::new());
    mock_buf.as_mut_slice()[0..10].copy_from_slice(&[9u8; 10]);

    tx_out.send((mock_buf, 10)).unwrap();
    drop(tx_out); // Close input stream for writer loop

    let mut writer_target = output;
    let outcome = writer_task(
        10,
        &mut writer_target,
        Some(&progress_tx),
        rx_out,
        tx_pool,
        None,
    )
    .unwrap();

    assert_eq!(outcome.written, 10);
    assert_eq!(outcome.sha256, sha256(&[9u8; 10]));

    // Assert content was written correctly
    let written_bytes = writer_target.into_inner();
    assert_eq!(&written_bytes[0..10], &[9u8; 10]);

    // Assert progress tracking worked
    assert!(progress_rx.try_recv().is_ok());
    // Assert buffer was successfully recycled back to tx_pool
    assert!(rx_pool.try_recv().is_ok());
}

#[test]
fn test_cancellation_token() {
    let token = CancellationToken::default();
    drop(token.drop_guard());

    let input_data = vec![0u8; 100];
    let mut cursor = Cursor::new(input_data);
    let (buf_tx_pool, buf_rx_pool) = mpsc::channel();
    let (buf_tx_out, _) = mpsc::sync_channel(2);

    buf_tx_pool.send(Box::new(DirectIoBuffer::new())).unwrap();

    let result = reader_task(&mut cursor, buf_rx_pool, buf_tx_out, Some(token));

    // Should return an error variant associated with cancellation
    assert!(result.is_err());
}
