use std::{
    io::Read,
    time::{Duration, Instant},
};

use bb_helper::cancel::CancellationToken;
use sha2::{Digest as _, Sha256};

use crate::{
    Error, Result, check_cancel,
    model::{
        DfuDevice, DfuProgress, DfuStage, DfuStageInput, DfuStageKind, DfuState,
        DfuTerminalEvidence, FlashReport, StageReport, TransportErrorKind, UsbPath,
    },
    transport::DfuTransport,
};

const BOOT_STATUS_TIMEOUT: Duration = Duration::from_secs(15);
const RAW_STATUS_TIMEOUT: Duration = Duration::from_secs(300);
const DETACH_TIMEOUT: Duration = Duration::from_secs(1);
/// How long a board gets to leave the bus after the host resets it.
const PORT_CLEAR_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy)]
struct BlockCounter {
    current: u16,
    wraps: u64,
}

impl BlockCounter {
    const fn new() -> Self {
        Self {
            current: 0,
            wraps: 0,
        }
    }

    fn take(&mut self) -> u16 {
        let block = self.current;
        let (next, wrapped) = self.current.overflowing_add(1);
        self.current = next;
        self.wraps += u64::from(wrapped);
        block
    }
}

/// Execute an already resolved four-stage T3 DFU plan.
///
/// Every input is streamed once, hashed while streaming, and compared with the typed integrity
/// metadata. The raw image is never buffered in full. Progress is reported per phase — see
/// [`DfuProgress`] — rather than as one number that would have to pretend the unmeasurable waits
/// are measurable.
pub fn flash_with_transport<T: DfuTransport>(
    transport: &mut T,
    mut inputs: Vec<DfuStageInput>,
    vendor_id: u16,
    product_id: u16,
    path: UsbPath,
    mut progress: Option<&mut dyn FnMut(DfuProgress)>,
    cancel: Option<&CancellationToken>,
) -> Result<FlashReport> {
    validate_plan(&inputs)?;
    // Still summed, and still refused on overflow: a plan whose sizes cannot be added up is a plan
    // whose stages cannot be held to a byte count.
    inputs.iter().try_fold(0_u64, |sum, input| {
        let size = input
            .stage
            .expected_size
            .ok_or_else(|| Error::MissingSize {
                artifact: input.stage.artifact_name.clone(),
            })?;
        sum.checked_add(size).ok_or(Error::ProgressSizeOverflow)
    })?;
    let mut reports = Vec::with_capacity(inputs.len());
    let mut prefetched_device: Option<DfuDevice> = None;

    for (index, mut input) in inputs.drain(..).enumerate() {
        check_cancel(cancel)?;
        let device = if let Some(device) = prefetched_device.take() {
            if !device
                .alt_settings
                .iter()
                .any(|alt| alt == &input.stage.alt_setting)
            {
                return Err(Error::WrongAltSetting {
                    expected: input.stage.alt_setting.clone(),
                    available: device.alt_settings,
                });
            }
            device
        } else {
            // Nothing to count while the board enumerates, so the phase is named instead.
            report(&mut progress, DfuProgress::Reconnecting);
            let deadline = Instant::now() + input.stage.reconnect_timeout;
            transport.wait_for_alt(
                vendor_id,
                product_id,
                &path,
                &input.stage.alt_setting,
                deadline,
                cancel,
            )?
        };
        tracing::info!(
            artifact = %input.stage.artifact_name,
            alt_setting = %input.stage.alt_setting,
            expected_size = input.stage.expected_size,
            available_alts = ?device.alt_settings,
            "DFU stage starting"
        );
        transport.claim(&device, &input.stage.alt_setting)?;
        prepare_interface(transport)?;

        let transfer = stream_stage(
            transport,
            &input.stage,
            input.reader.as_mut(),
            // 1-based for display; the raw stage reports itself and ignores this.
            (index + 1) as u8,
            &mut progress,
            cancel,
        );

        let (bytes_sent, connected) = match transfer {
            Ok(result) => result,
            Err(error) => {
                let _ = transport.release();
                return Err(error);
            }
        };

        tracing::info!(
            artifact = %input.stage.artifact_name,
            bytes_sent,
            still_connected = connected,
            "DFU stage transferred"
        );

        let terminal_evidence = match &input.stage.kind {
            DfuStageKind::BootArtifact { next_alt_setting } => {
                // The final zero-length `DNLOAD` only *requests* the end of the transfer. Per DFU
                // 1.1 the device leaves dfuDNLOAD-SYNC for dfuMANIFEST-SYNC when the host polls
                // `GET_STATUS`, and manifestation is where the TI ROM acts on the artifact it just
                // received. Skipping the poll leaves the ROM parked mid-transaction: it never
                // treats the image as complete, never boots it, and the wait below then times out
                // no matter what else the host sends.
                if connected {
                    let manifest = wait_for_boot_manifest(transport)?;
                    tracing::info!(
                        artifact = %input.stage.artifact_name,
                        ?manifest,
                        "boot artifact manifested"
                    );
                    match manifest {
                        // Still on the bus after manifestation means the device is not going to
                        // leave on its own. The TI ROM does (it jumps straight into the artifact,
                        // which is why it reports `Disconnected`), but U-Boot's DFU gadget sits in
                        // its download loop until the host lets go, and only then boots what it
                        // was given — this is exactly why the documented flow drives it with
                        // `dfu-util -R`. dfuMANIFEST-WAIT-RESET additionally *requires* the reset
                        // per DFU 1.1, so both endings are handled the same way.
                        BootManifest::Idle | BootManifest::WaitReset => {
                            // `dfu-util -R` is DETACH *then* reset, and the order is not cosmetic.
                            // U-Boot's gadget leaves its download loop from the detach trigger the
                            // DETACH request sets; a bare bus reset only restarts the gadget, so
                            // the board stays in DFU, never boots the artifact, and the next stage
                            // writes into the instance that was supposed to have left.
                            detach_before_reset(transport, &input.stage.artifact_name);
                            tolerate_reset_disconnect(transport.reset())?;
                            // The reset invalidates the handle, so releasing is best effort; the
                            // session is dropped either way.
                            let _ = transport.release();
                            // Without this the next lookup can match the instance that is on its
                            // way out: the R5 SPL publishes `tispl.bin` and `u-boot.img` at the
                            // same time, so the alt-setting alone cannot tell the departing stage
                            // from the one that replaces it.
                            if !transport.wait_for_port_clear(
                                vendor_id,
                                product_id,
                                &path,
                                Instant::now() + PORT_CLEAR_TIMEOUT,
                                cancel,
                            )? {
                                tracing::warn!(
                                    artifact = %input.stage.artifact_name,
                                    "board stayed on the bus after the reset"
                                );
                            }
                        }
                        BootManifest::Disconnected => {
                            tolerate_reset_disconnect(transport.release())?;
                        }
                    }
                }

                report(&mut progress, DfuProgress::Reconnecting);
                let next = transport.wait_for_alt(
                    vendor_id,
                    product_id,
                    &path,
                    next_alt_setting,
                    Instant::now() + input.stage.reconnect_timeout,
                    cancel,
                )?;
                if next.path != device.path {
                    return Err(Error::DevicePathChanged {
                        expected: device.path,
                        actual: next.path,
                    });
                }
                prefetched_device = Some(next);
                DfuTerminalEvidence::NextAltEnumerated(next_alt_setting.clone())
            }
            DfuStageKind::RawEmmc => {
                // Once the ZLP has started U-Boot's eMMC flush, cancellation is deliberately not
                // checked: interrupting the manifest transaction is not a safe cancellation point.
                // This is also the longest phase with nothing to count, so it is named loudly.
                report(&mut progress, DfuProgress::Finalizing);
                let evidence = wait_for_manifest(transport)?;
                transport
                    .detach(DETACH_TIMEOUT)
                    .map_err(|source| Error::FinalDetach {
                        source: Box::new(source),
                    })?;
                // A successful DETACH request may make release observe NO_DEVICE. The request
                // itself is the required evidence; only reset-style disconnect is accepted here.
                tolerate_reset_disconnect(transport.release())?;
                evidence
            }
        };

        reports.push(StageReport {
            artifact_name: input.stage.artifact_name,
            bytes_sent,
            terminal_evidence,
        });
    }

    Ok(FlashReport { stages: reports })
}

fn report(progress: &mut Option<&mut dyn FnMut(DfuProgress)>, event: DfuProgress) {
    if let Some(callback) = progress.as_deref_mut() {
        callback(event);
    }
}

fn validate_plan(inputs: &[DfuStageInput]) -> Result<()> {
    if inputs.len() != 4 {
        return Err(Error::InvalidPlan(format!(
            "T3 DFU requires exactly four stages, got {}",
            inputs.len()
        )));
    }
    for (index, input) in inputs.iter().enumerate() {
        if input.stage.expected_size == Some(0) {
            return Err(Error::EmptyImage(input.stage.artifact_name.clone()));
        }
        match (&input.stage.kind, inputs.get(index + 1)) {
            (DfuStageKind::BootArtifact { next_alt_setting }, Some(next)) => {
                if next_alt_setting != &next.stage.alt_setting {
                    return Err(Error::InvalidPlan(format!(
                        "stage `{}` expects next alt `{next_alt_setting}`, but the next stage uses `{}`",
                        input.stage.artifact_name, next.stage.alt_setting
                    )));
                }
            }
            (DfuStageKind::RawEmmc, None) => {}
            (DfuStageKind::RawEmmc, Some(_)) => {
                return Err(Error::InvalidPlan(
                    "raw eMMC must be the final stage".to_owned(),
                ));
            }
            (DfuStageKind::BootArtifact { .. }, None) => {
                return Err(Error::InvalidPlan(
                    "the final stage must be raw eMMC".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn prepare_interface<T: DfuTransport>(transport: &mut T) -> Result<()> {
    let mut status = transport.status()?;
    if status.state == DfuState::DfuError {
        transport.clear_status()?;
        status = transport.status()?;
    }
    if matches!(
        status.state,
        DfuState::DfuDnloadIdle | DfuState::DfuUploadIdle
    ) {
        transport.abort()?;
        status = transport.status()?;
    }
    if status.status != 0 || !matches!(status.state, DfuState::DfuIdle | DfuState::DfuDnloadIdle) {
        return Err(Error::UnexpectedState {
            context: "preparing DFU interface",
            state: status.state,
            status: status.status,
        });
    }
    Ok(())
}

/// Returns `(bytes_sent, interface_still_connected)`.
fn stream_stage<T: DfuTransport>(
    transport: &mut T,
    stage: &DfuStage,
    reader: &mut dyn Read,
    stage_index: u8,
    progress: &mut Option<&mut dyn FnMut(DfuProgress)>,
    cancel: Option<&CancellationToken>,
) -> Result<(u64, bool)> {
    let expected_size = stage.expected_size.ok_or_else(|| Error::MissingSize {
        artifact: stage.artifact_name.clone(),
    })?;
    if expected_size == 0 {
        return Err(Error::EmptyImage(stage.artifact_name.clone()));
    }
    let transfer_size = transport.transfer_size()?;
    if transfer_size == 0 || transfer_size > usize::from(u16::MAX) {
        return Err(Error::InvalidTransferSize(transfer_size));
    }

    let mut buffer = vec![0_u8; transfer_size];
    let mut hasher = Sha256::new();
    let mut sent = 0_u64;
    let mut blocks = BlockCounter::new();
    let mut connected = true;

    while sent < expected_size {
        check_cancel(cancel)?;
        let wanted = usize::try_from((expected_size - sent).min(transfer_size as u64))
            .expect("bounded by transfer_size");
        let read = reader
            .read(&mut buffer[..wanted])
            .map_err(|source| Error::ImageRead {
                artifact: stage.artifact_name.clone(),
                source,
            })?;
        if read == 0 {
            return Err(Error::ShortImage {
                artifact: stage.artifact_name.clone(),
                expected: expected_size,
                actual: sent,
            });
        }
        hasher.update(&buffer[..read]);
        let block = blocks.take();
        let written = transport
            .download_chunk(block, &buffer[..read])
            .map_err(|error| Error::StageTransfer {
                artifact: stage.artifact_name.clone(),
                source: Box::new(error),
            })?;
        if written != read {
            return Err(Error::ShortUsbWrite {
                expected: read,
                actual: written,
            });
        }
        sent += read as u64;

        if let Err(error) = wait_for_download_idle(
            transport,
            match stage.kind {
                DfuStageKind::BootArtifact { .. } => BOOT_STATUS_TIMEOUT,
                DfuStageKind::RawEmmc => RAW_STATUS_TIMEOUT,
            },
            cancel,
        ) {
            if is_boot_reset_disconnect(stage, &error) && sent == expected_size {
                connected = false;
                break;
            }
            return Err(Error::StageTransfer {
                artifact: stage.artifact_name.clone(),
                source: Box::new(error),
            });
        }

        // Each stage reports its own fraction. The raw image dwarfs the three boot artifacts, so
        // folding them into one bar would make the first 3 % of the work occupy three quarters of
        // the visible motion and the remaining 97 % look like a hang.
        let fraction = sent as f32 / expected_size as f32;
        report(
            progress,
            match stage.kind {
                DfuStageKind::BootArtifact { .. } => DfuProgress::BootStage {
                    index: stage_index,
                    fraction,
                },
                DfuStageKind::RawEmmc => DfuProgress::RawWrite(fraction),
            },
        );
    }

    let mut extra = [0_u8; 1];
    if reader.read(&mut extra).map_err(|source| Error::ImageRead {
        artifact: stage.artifact_name.clone(),
        source,
    })? != 0
    {
        return Err(Error::LongImage {
            artifact: stage.artifact_name.clone(),
            expected: expected_size,
        });
    }

    let actual_hash: [u8; 32] = hasher.finalize().into();
    if actual_hash != stage.expected_sha256 {
        return Err(Error::StageHashMismatch {
            artifact: stage.artifact_name.clone(),
            expected: stage.expected_sha256,
            actual: actual_hash,
        });
    }

    if connected {
        let finish_block = blocks.take();
        if let Err(error) = transport.finish_download(finish_block) {
            if is_boot_reset_disconnect(stage, &error) {
                connected = false;
            } else {
                return Err(Error::Zlp {
                    artifact: stage.artifact_name.clone(),
                    source: Box::new(error),
                });
            }
        }
    } else if matches!(stage.kind, DfuStageKind::RawEmmc) {
        return Err(Error::DisconnectedBeforeTerminalState);
    }

    Ok((sent, connected))
}

fn wait_for_download_idle<T: DfuTransport>(
    transport: &mut T,
    timeout: Duration,
    cancel: Option<&CancellationToken>,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        check_cancel(cancel)?;
        let status = transport.status()?;
        if status.status != 0 || status.state == DfuState::DfuError {
            return Err(Error::UnexpectedState {
                context: "polling a download chunk",
                state: status.state,
                status: status.status,
            });
        }
        match status.state {
            DfuState::DfuDnloadIdle => return Ok(()),
            DfuState::DfuDnloadSync | DfuState::DfuDnBusy => {}
            _ => {
                return Err(Error::UnexpectedState {
                    context: "polling a download chunk",
                    state: status.state,
                    status: status.status,
                });
            }
        }
        if Instant::now() >= deadline {
            return Err(Error::StatusTimeout("download chunk"));
        }
        transport.sleep(
            status
                .poll_timeout
                .max(Duration::from_millis(1))
                .min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

/// How a boot artifact's manifestation ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BootManifest {
    /// The device returned to dfuIDLE by itself — it declared itself manifestation tolerant.
    Idle,
    /// The device parked in dfuMANIFEST-WAIT-RESET and can only be freed by a USB reset.
    WaitReset,
    /// The device left the bus mid-manifestation, which for a boot artifact means it booted.
    Disconnected,
}

/// Drive a boot artifact through manifestation.
///
/// Unlike [`wait_for_manifest`] a disconnect here is the *expected* ending rather than a failure:
/// the ROM hands control to the artifact it just accepted, so the port drops before any final
/// status can be read.
fn wait_for_boot_manifest<T: DfuTransport>(transport: &mut T) -> Result<BootManifest> {
    let deadline = Instant::now() + BOOT_STATUS_TIMEOUT;
    loop {
        let status = match transport.status() {
            Ok(status) => status,
            Err(error)
                if transport_error_kind(&error)
                    .is_some_and(TransportErrorKind::may_be_reset_disconnect) =>
            {
                return Ok(BootManifest::Disconnected);
            }
            Err(error) => return Err(error),
        };
        if status.status != 0 || status.state == DfuState::DfuError {
            return Err(Error::UnexpectedState {
                context: "manifesting a boot artifact",
                state: status.state,
                status: status.status,
            });
        }
        match status.state {
            DfuState::DfuIdle => return Ok(BootManifest::Idle),
            DfuState::DfuManifestWaitReset => return Ok(BootManifest::WaitReset),
            DfuState::DfuManifestSync | DfuState::DfuManifest | DfuState::DfuDnloadSync => {}
            _ => {
                return Err(Error::UnexpectedState {
                    context: "manifesting a boot artifact",
                    state: status.state,
                    status: status.status,
                });
            }
        }
        if Instant::now() >= deadline {
            return Err(Error::StatusTimeout("boot artifact manifest"));
        }
        transport.sleep(
            status
                .poll_timeout
                .max(Duration::from_millis(1))
                .min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

fn wait_for_manifest<T: DfuTransport>(transport: &mut T) -> Result<DfuTerminalEvidence> {
    let deadline = Instant::now() + RAW_STATUS_TIMEOUT;
    loop {
        let status = transport.status().map_err(|error| {
            if transport_error_kind(&error).is_some_and(TransportErrorKind::may_be_reset_disconnect)
            {
                Error::DisconnectedBeforeTerminalState
            } else {
                error
            }
        })?;
        if status.status != 0 || status.state == DfuState::DfuError {
            return Err(Error::UnexpectedState {
                context: "waiting for raw eMMC manifest",
                state: status.state,
                status: status.status,
            });
        }
        match status.state {
            DfuState::DfuIdle => return Ok(DfuTerminalEvidence::DfuIdle),
            DfuState::DfuManifestWaitReset => {
                return Ok(DfuTerminalEvidence::ManifestWaitReset);
            }
            DfuState::DfuManifestSync | DfuState::DfuManifest | DfuState::DfuDnloadSync => {}
            _ => {
                return Err(Error::UnexpectedState {
                    context: "waiting for raw eMMC manifest",
                    state: status.state,
                    status: status.status,
                });
            }
        }
        if Instant::now() >= deadline {
            return Err(Error::StatusTimeout("raw eMMC manifest"));
        }
        transport.sleep(
            status
                .poll_timeout
                .max(Duration::from_millis(1))
                .min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

fn is_boot_reset_disconnect(stage: &DfuStage, error: &Error) -> bool {
    matches!(stage.kind, DfuStageKind::BootArtifact { .. })
        && stage.reset_after
        && transport_error_kind(error).is_some_and(TransportErrorKind::may_be_reset_disconnect)
}

fn transport_error_kind(error: &Error) -> Option<TransportErrorKind> {
    match error {
        Error::Transport(source) => Some(source.kind),
        _ => None,
    }
}

/// Send the `DFU_DETACH` that lets a U-Boot DFU gadget leave its download loop, then let the
/// caller reset.
///
/// A failure here is logged and swallowed on purpose. A device that already left the bus, or one
/// that stalls a request the DFU 1.1 state table only defines for appIDLE, is still driven out by
/// the reset that follows — refusing would fail writes the reference `dfu-util -R` flow completes.
fn detach_before_reset<T: DfuTransport>(transport: &mut T, artifact: &str) {
    if let Err(error) = transport.detach(DETACH_TIMEOUT) {
        tracing::warn!(
            artifact,
            %error,
            "DFU detach before reset was refused; resetting anyway"
        );
    }
}

fn tolerate_reset_disconnect(result: Result<()>) -> Result<()> {
    match result {
        Err(Error::Transport(source)) if source.kind.may_be_reset_disconnect() => Ok(()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_counter_wrap_is_explicit_and_deterministic() {
        let mut counter = BlockCounter {
            current: u16::MAX - 1,
            wraps: 0,
        };
        assert_eq!(counter.take(), u16::MAX - 1);
        assert_eq!(counter.take(), u16::MAX);
        assert_eq!(counter.take(), 0);
        assert_eq!(counter.wraps, 1);
    }
}
