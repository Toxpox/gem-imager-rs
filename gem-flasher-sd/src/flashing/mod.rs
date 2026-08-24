use std::io::{Read, Seek, Write};
use std::sync::mpsc;
use std::time::Instant;

use gem_helper::cancel::CancellationToken;
use sha2::{Digest as _, Sha256};

use crate::Result;
use crate::customization::Customization;
// `Commit` is reachable through the `Eject: Commit` supertrait bound, so it needs no import here.
use crate::helpers::{
    DirectIoBuffer, Eject, PublishLayout, chan_send, check_cancel, progress, read_at_least,
};

#[cfg(test)]
mod tests;

// Stack overflow occurs during debug since box moves data from stack to heap in debug builds
#[cfg(not(debug_assertions))]
const BUFFER_SIZE: usize = 1024 * 1024;
#[cfg(debug_assertions)]
const BUFFER_SIZE: usize = 8 * 1024;

/// Direct IO wants reads whose length is a multiple of the device block size, so the tail of the
/// read-back is rounded up to this. `BUFFER_SIZE` is a multiple of it in both build profiles.
const IO_ALIGNMENT: usize = 4096;

/// Which stage of the flash a progress value belongs to.
///
/// The stages are reported separately rather than folded into one 0..1 bar. A full read-back costs
/// roughly as much time as the write it verifies, and a bar that reaches 100% and then sits there
/// for another minute is exactly what makes a user pull the card early.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Status {
    Preparing,
    /// Fraction of the image written to the device.
    Writing(f32),
    /// Fraction of the written region read back and hashed.
    Verifying(f32),
    Customizing,
}

/// What actually reached the device, as counted by the writer itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WriteOutcome {
    /// Bytes handed to the device, including the zero padding `read_aligned` adds to the last
    /// chunk. This is the region the read-back covers.
    written: u64,
    /// SHA-256 over exactly those `written` bytes.
    sha256: [u8; 32],
}

fn reader_task(
    mut img: impl Read,
    buf_rx: mpsc::Receiver<Box<DirectIoBuffer<BUFFER_SIZE>>>,
    buf_tx: mpsc::SyncSender<(Box<DirectIoBuffer<BUFFER_SIZE>>, usize)>,
    cancel: Option<CancellationToken>,
) -> Result<()> {
    while let Ok(mut buf) = buf_rx.recv() {
        let count = read_aligned(&mut img, buf.as_mut_slice())?;
        if count == 0 {
            break;
        }

        buf_tx
            .send((buf, count))
            .map_err(|_| crate::Error::WriterClosed)?;
        check_cancel(cancel.as_ref())?;
    }

    Ok(())
}

/// Writes the decoded stream to the device, counting and hashing every byte on the way past.
///
/// The hash is taken here rather than on the reader side because the point of comparison is what
/// the *writer* claims to have put on the device: read-back then answers "did the device keep what
/// I handed it", independently of whether the image itself was the right one (which the decoder's
/// own extract gate answers).
fn writer_task(
    img_size: u64,
    mut sd: impl Write + Seek,
    chan: Option<&mpsc::SyncSender<Status>>,
    buf_rx: mpsc::Receiver<(Box<DirectIoBuffer<BUFFER_SIZE>>, usize)>,
    buf_tx: mpsc::SyncSender<Box<DirectIoBuffer<BUFFER_SIZE>>>,
    cancel: Option<CancellationToken>,
) -> Result<WriteOutcome> {
    let mut pos = 0u64;
    let mut hasher = Sha256::new();

    while let Ok((buf, count)) = buf_rx.recv() {
        let chunk = &buf.as_slice()[..count];

        // `write_all` turns a short write into an error; relaxing it to `write` would drop the tail.
        sd.write_all(chunk)?;
        hasher.update(chunk);

        pos += count as u64;
        chan_send(chan, Status::Writing(progress(pos, img_size)));

        let _ = buf_tx.send(buf);
        check_cancel(cancel.as_ref())?;
    }

    sd.flush()?;

    // The reader pads the final chunk to 512-byte alignment, so the writer may overshoot but never
    // undershoot. Fewer bytes than the image declares means the stream ended early.
    if pos < img_size {
        return Err(crate::Error::ShortWrite {
            expected: img_size,
            written: pos,
        });
    }

    Ok(WriteOutcome {
        written: pos,
        sha256: hasher.finalize().into(),
    })
}

/// A lot of reads from compressed files are not aligned. Since reading even from compressed files
/// is significantly faster than writing to SD Card, better to do multiple reads.
fn read_aligned(mut img: impl Read, buf: &mut [u8]) -> Result<usize> {
    const ALIGNMENT: usize = 512;

    let mut pos = 0;

    while pos != buf.len() {
        let count = img.read(&mut buf[pos..])?;
        if count == 0 {
            if pos % ALIGNMENT != 0 {
                let end = pos - pos % ALIGNMENT + ALIGNMENT;
                buf[pos..end].fill(0);
                pos = end;
            }
            return Ok(pos);
        }
        pos += count;
    }

    Ok(pos)
}

fn write_sd(
    img: impl Read + Send,
    img_size: u64,
    sd: impl Write + Seek,
    chan: Option<&mpsc::SyncSender<Status>>,
    cancel: Option<CancellationToken>,
) -> Result<WriteOutcome> {
    const NUM_BUFFERS: usize = 4;

    let (tx1, rx1) = std::sync::mpsc::sync_channel(NUM_BUFFERS);
    let (tx2, rx2) = std::sync::mpsc::sync_channel(NUM_BUFFERS);
    let global_start = Instant::now();

    for _ in 0..NUM_BUFFERS {
        tx1.send(Box::new(DirectIoBuffer::new())).unwrap();
    }

    std::thread::scope(|s| {
        let cancle_clone = cancel.clone();
        let handle = s.spawn(move || reader_task(img, rx1, tx2, cancle_clone));

        let write_res = writer_task(img_size, sd, chan, rx2, tx1, cancel);
        tracing::info!("Total Time taken: {:?}", global_start.elapsed());

        // The reader's error is reported first because it is the root cause: a decoder that fails its
        // integrity gate closes the channel, which the writer would surface only as `ShortWrite`.
        handle.join().unwrap()?;
        write_res
    })
}

/// Reads the written region back off the device and compares it with what the writer produced.
///
/// This is the only check that covers the device itself — controller-level write caching, a card
/// lying about its capacity, and a cable that dropped mid-transfer all survive every earlier gate
/// and fail here. It runs after the sync and before customization, so it sees the raw image exactly
/// as it was written.
fn verify_written(
    mut sd: impl Read + Seek,
    outcome: WriteOutcome,
    chan: Option<&mpsc::SyncSender<Status>>,
    cancel: Option<&CancellationToken>,
) -> Result<()> {
    sd.rewind()?;

    let mut buf = Box::new(DirectIoBuffer::<BUFFER_SIZE>::new());
    let mut hasher = Sha256::new();
    let mut pos = 0u64;

    chan_send(chan, Status::Verifying(0.0));

    while pos < outcome.written {
        check_cancel(cancel)?;

        let needed = std::cmp::min(BUFFER_SIZE as u64, outcome.written - pos) as usize;
        let request = needed.next_multiple_of(IO_ALIGNMENT).min(BUFFER_SIZE);

        read_at_least(&mut sd, &mut buf.as_mut_slice()[..request], needed)?;
        hasher.update(&buf.as_slice()[..needed]);

        pos += needed as u64;
        chan_send(chan, Status::Verifying(progress(pos, outcome.written)));
    }

    let actual: [u8; 32] = hasher.finalize().into();
    if actual != outcome.sha256 {
        return Err(crate::Error::ReadBackMismatch {
            expected: const_hex::encode(outcome.sha256).into(),
            actual: const_hex::encode(actual).into(),
        });
    }

    Ok(())
}

/// Refuse targets that are obviously wrong before anything is opened, and report the capacity the
/// image has to fit into.
///
/// `is_removable` on its own is not the check: USB-attached system disks and internal card readers
/// each report as removable on at least one platform, so the drive list's own system-disk
/// determination is what gates the write.
fn guard_target(path: &std::path::Path) -> Result<Option<u64>> {
    let dev = crate::devices(false).into_iter().find(|d| d.path == path);

    if dev.is_none() {
        // Enumeration missing the device is not evidence that writing to it is safe. The write path
        // is about to hand the path to `Clear-Disk` on Windows and to a raw `O_DIRECT` handle
        // elsewhere, so an unrecognised target skips both the system-disk and the capacity gate.
        // The format path already refuses this case (`helpers::destination_size`); refusing here
        // too keeps one policy across the crate.
        return Err(crate::Error::UnknownDestination {
            path: path.display().to_string().into_boxed_str(),
        });
    }

    evaluate_target(dev.as_ref())
}

/// The target decision itself, separated from device enumeration so it can be tested without real
/// hardware.
fn evaluate_target(dev: Option<&crate::Device>) -> Result<Option<u64>> {
    let Some(dev) = dev else {
        return Ok(None);
    };

    if dev.is_system {
        return Err(crate::Error::SystemDisk {
            name: dev.name.clone().into(),
        });
    }

    // A zero size means the backend reported none; treat that as unknown, not as too small.
    Ok(Some(dev.size).filter(|s| *s > 0))
}

/// Flash OS image to SD card.
///
/// # Customization
///
/// Support post flashing customization. Currently only sysconf is supported, which is used by
/// [BeagleBoard.org].
///
/// # Image
///
/// Using a resolver function for image and image size. This is to allow downloading the image, or
/// some kind of lazy loading after SD card permissions have be acquired. This is useful in GUIs
/// since the user would expect a password prompt at the start of flashing.
///
/// Many users might switch task after starting the flashing process, which would make it
/// frustrating if the prompt occured after downloading.
///
/// The resolver is also the non-destructive integrity boundary. It must not return until every
/// source-side validation that can fail at end-of-stream, such as archive and extracted-image
/// hashes, has completed. Once it succeeds and the capacity/cancellation checks pass, flashing may
/// hide the destination's existing partition layout before reading from the returned reader.
///
/// # Progress
///
/// Each [`Status`] stage carries its own 0..1 progress; the stages do not share one bar.
///
/// # Verification
///
/// The written region is always read back off the device and compared against the bytes the writer
/// produced. A mismatch is [`crate::Error::ReadBackMismatch`] — never a warning.
pub fn flash<'a, R, C>(
    img: impl FnOnce() -> std::io::Result<(R, u64)> + Send,
    dst: crate::Destination,
    chan: Option<mpsc::SyncSender<Status>>,
    customizations: impl Iterator<Item = Customization<C>> + Send,
    cancel: Option<CancellationToken>,
) -> Result<()>
where
    R: Read + Send,
    C: Iterator<Item = (Box<str>, crate::ContentType<'a>)> + Send,
{
    tracing::info!("Opening Destination");

    match dst {
        crate::Destination::File(path) => {
            let sd = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(path)?;
            flash_internal(img, sd, None, chan, customizations, cancel)
        }
        crate::Destination::SdCard(path) => {
            let capacity = guard_target(&path)?;
            let sd = crate::pal::open(&path)?;
            let sd = crate::helpers::SdCardWrapper::new(sd);
            flash_internal(img, sd, capacity, chan, customizations, cancel)
        }
    }
}

fn flash_internal<'a, R, Sd, C>(
    img: impl FnOnce() -> std::io::Result<(R, u64)> + Send,
    mut sd: Sd,
    capacity: Option<u64>,
    chan: Option<mpsc::SyncSender<Status>>,
    customizations: impl Iterator<Item = Customization<C>> + Send,
    cancel: Option<CancellationToken>,
) -> Result<()>
where
    R: Read + Send,
    Sd: Read + Write + Seek + Eject + PublishLayout + std::fmt::Debug,
    C: Iterator<Item = (Box<str>, crate::ContentType<'a>)> + Send,
{
    tracing::info!("Resolving Image");

    let chan = chan.as_ref();
    chan_send(chan, Status::Preparing);
    check_cancel(cancel.as_ref())?;

    let (img, img_size) = match img() {
        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
            return Err(crate::Error::Aborted);
        }
        result => result?,
    };

    // Checked before the first write: a card that runs out halfway wastes an hour first.
    if let Some(available) = capacity
        && img_size > available
    {
        return Err(crate::Error::InsufficientCapacity {
            required: img_size,
            available,
        });
    }

    check_cancel(cancel.as_ref())?;

    // Everything above this line can still fail without touching the destination: image resolution
    // (which is where a download and its integrity gates run), the capacity check, and the first
    // cancellation point. Wiping the existing partition table is irreversible, so it happens only
    // once the flash is actually committed to writing.
    sd.hide_existing_layout(capacity)?;

    tracing::info!("Writing to SD Card");
    let outcome = write_sd(img, img_size, &mut sd, chan, cancel.clone())?;

    tracing::info!("Syncing {} bytes to the device", outcome.written);
    sd.commit()
        .map_err(|source| crate::Error::SyncFailed { source })?;

    tracing::info!("Verifying written data");
    verify_written(&mut sd, outcome, chan, cancel.as_ref())?;

    tracing::info!("Applying customization");
    chan_send(chan, Status::Customizing);
    let mut dev = crate::helpers::DeviceWrapper::new(sd)?;
    for c in customizations {
        check_cancel(cancel.as_ref())?;
        c.customize(&mut dev, cancel.clone())?;
    }

    let mut sd = dev.into_inner();
    sd.commit()
        .map_err(|source| crate::Error::SyncFailed { source })?;

    tracing::info!("Publishing and verifying the partition layout");
    sd.publish_layout()?;

    // Everything is durable by now, so a refused eject is a convenience problem, not a data one.
    tracing::info!("Ejecting SD Card");
    if let Err(e) = sd.eject() {
        tracing::warn!("Failed to eject the destination: {e}");
    }

    Ok(())
}
