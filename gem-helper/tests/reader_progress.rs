#![cfg(feature = "reader_progress")]

use std::io::{Read, Seek, SeekFrom};
use std::sync::mpsc;

use gem_helper::reader_progress::ReaderWithProgress;

#[test]
fn test_happy_path_progress() {
    let data = vec![0u8; 100];
    let (tx, rx) = mpsc::sync_channel(10);

    let mut reader = ReaderWithProgress::new(std::io::Cursor::new(data), 100, Some(tx));
    let mut buf = vec![0u8; 25];

    assert!(reader.read(&mut buf).is_ok());
    assert_eq!(rx.try_recv().unwrap(), 0.25);

    assert!(reader.read(&mut buf).is_ok());
    assert_eq!(rx.try_recv().unwrap(), 0.50);
}

#[test]
fn test_progress_tracks_absolute_position_after_seek() {
    let data = vec![0u8; 100];
    let (tx, rx) = mpsc::sync_channel(10);

    let mut reader = ReaderWithProgress::new(std::io::Cursor::new(data), 100, Some(tx));
    let mut buf = vec![0u8; 10];

    let count = reader.read(&mut buf).unwrap();
    assert_eq!(count, 10);
    assert_eq!(rx.try_recv().unwrap(), 0.10);

    reader.seek(SeekFrom::Current(40)).unwrap();

    let count = reader.read(&mut buf).unwrap();
    assert_eq!(count, 10);

    let reported_progress = rx.try_recv().unwrap();

    assert_eq!(
        reported_progress, 0.60,
        "Progress should track absolute position after a seek, got {}",
        reported_progress
    );
}

#[test]
fn test_zero_size_handling() {
    let data = vec![];
    let (tx, rx) = mpsc::sync_channel(10);

    let mut reader = ReaderWithProgress::new(std::io::Cursor::new(data), 0, Some(tx));
    let mut buf = vec![0u8; 10];

    let _ = reader.read(&mut buf);

    if let Ok(progress) = rx.try_recv() {
        assert!(!progress.is_nan(), "Progress emitted NaN!");
    }
}

#[test]
fn test_dropped_receiver_does_not_panic() {
    let data = vec![0u8; 10];
    let (tx, rx) = mpsc::sync_channel(1);

    let mut reader = ReaderWithProgress::new(std::io::Cursor::new(data), 10, Some(tx));
    let mut buf = vec![0u8; 5];

    drop(rx);

    assert!(reader.read(&mut buf).is_ok());
}
