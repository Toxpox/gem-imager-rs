use std::io::{Read, Seek, Write};
use std::sync::mpsc;
use std::time::Instant;

use gem_helper::cancel::CancellationToken;
use sha2::{Digest as _, Sha256};

use crate::Result;
use crate::customization::Customization;
use crate::helpers::{
    DirectIoBuffer, Eject, PublishLayout, chan_send, check_cancel, progress, read_at_least,
};

#[cfg(test)]
mod tests;

#[cfg(not(debug_assertions))]
const BUFFER_SIZE: usize = 1024 * 1024;
#[cfg(debug_assertions)]
const BUFFER_SIZE: usize = 8 * 1024;

const IO_ALIGNMENT: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Status {
    Preparing,
    Writing(f32),
    Verifying(f32),
    Customizing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WriteOutcome {
    written: u64,
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

        sd.write_all(chunk)?;
        hasher.update(chunk);

        pos += count as u64;
        chan_send(chan, Status::Writing(progress(pos, img_size)));

        let _ = buf_tx.send(buf);
        check_cancel(cancel.as_ref())?;
    }

    sd.flush()?;

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

        handle.join().unwrap()?;
        write_res
    })
}

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

fn guard_target(path: &std::path::Path, expected: &crate::DeviceIdentity) -> Result<Option<u64>> {
    let dev = crate::devices(false).into_iter().find(|d| d.path == path);

    if dev.is_none() {
        return Err(crate::Error::UnknownDestination {
            path: path.display().to_string().into_boxed_str(),
        });
    }

    evaluate_target(dev.as_ref(), expected)
}

fn evaluate_target(
    dev: Option<&crate::Device>,
    expected: &crate::DeviceIdentity,
) -> Result<Option<u64>> {
    let Some(dev) = dev else {
        return Ok(None);
    };

    if dev.is_system {
        return Err(crate::Error::SystemDisk {
            name: dev.name.clone().into(),
        });
    }

    if !dev.identity.matches(expected) {
        return Err(crate::Error::DestinationChanged {
            name: dev.name.clone().into(),
        });
    }

    Ok(Some(dev.size).filter(|s| *s > 0))
}

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
        crate::Destination::SdCard(path, identity) => {
            let capacity = guard_target(&path, &identity)?;
            let sd = crate::pal::open(&path)?;
            let confirmed = guard_target(&path, &identity)?;

            if confirmed != capacity {
                return Err(crate::Error::DestinationChanged {
                    name: path.display().to_string().into_boxed_str(),
                });
            }

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

    if let Some(available) = capacity
        && img_size > available
    {
        return Err(crate::Error::InsufficientCapacity {
            required: img_size,
            available,
        });
    }

    check_cancel(cancel.as_ref())?;

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

    tracing::info!("Ejecting SD Card");
    if let Err(e) = sd.eject() {
        tracing::warn!("Failed to eject the destination: {e}");
    }

    Ok(())
}
