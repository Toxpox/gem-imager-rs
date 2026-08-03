use std::{
    collections::VecDeque,
    io::Cursor,
    time::{Duration, Instant},
};

use bb_flasher_dfu::{
    DfuDevice, DfuProgress, DfuStage, DfuStageInput, DfuStageKind, DfuState, DfuStatus,
    DfuTerminalEvidence, DfuTransport, Error, TransportError, TransportErrorKind, UsbPath,
    flash_with_transport,
};
use bb_helper::cancel::CancellationToken;
use sha2::{Digest as _, Sha256};

const VID: u16 = 0x0451;
const PID: u16 = 0x6165;

#[derive(Debug)]
enum Event {
    Wait { alt: &'static str, address: u8 },
    Claim(&'static str),
    Release,
    TransferSize(usize),
    Status(DfuState, u8),
    Download { block: u16, bytes: usize },
    Finish(u16),
    Detach,
    Reset,
    Fail(&'static str, TransportErrorKind),
    Timeout(&'static str),
}

struct MockTransport {
    events: VecDeque<Event>,
    seen: Vec<String>,
}

impl MockTransport {
    fn new(events: Vec<Event>) -> Self {
        Self {
            events: events.into(),
            seen: Vec::new(),
        }
    }

    fn event(&mut self, operation: &'static str) -> Result<Event, Error> {
        match self
            .events
            .pop_front()
            .expect("mock event script exhausted")
        {
            Event::Fail(expected, kind) if expected == operation => Err(Error::Transport(
                TransportError::new(kind, format!("injected {operation} failure")),
            )),
            event => Ok(event),
        }
    }

    fn done(&self) {
        assert!(self.events.is_empty(), "unused events: {:?}", self.events);
    }
}

impl DfuTransport for MockTransport {
    fn enumerate(&mut self, _vendor_id: u16, _product_id: u16) -> Result<Vec<DfuDevice>, Error> {
        panic!("scripted wait_for_alt is used")
    }

    fn wait_for_alt(
        &mut self,
        _vendor_id: u16,
        _product_id: u16,
        path: &UsbPath,
        alt_setting: &str,
        _deadline: Instant,
        cancel: Option<&CancellationToken>,
    ) -> Result<DfuDevice, Error> {
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            return Err(Error::Aborted);
        }
        match self.event("wait")? {
            Event::Wait { alt, address } => {
                assert_eq!(alt, alt_setting);
                self.seen.push(format!("wait:{alt}"));
                Ok(device(path.clone(), alt, address))
            }
            Event::Timeout(alt) => Err(Error::ReconnectTimeout {
                alt_setting: alt.to_owned(),
                path: path.clone(),
            }),
            event => panic!("expected wait event, got {event:?}"),
        }
    }

    /// Scripted `wait_for_alt` already stands in for enumeration, so the settle gate has nothing
    /// to observe here; it is covered against a real enumeration in the transport's own tests.
    fn wait_for_port_clear(
        &mut self,
        _vendor_id: u16,
        _product_id: u16,
        _path: &UsbPath,
        _deadline: Instant,
        _cancel: Option<&CancellationToken>,
    ) -> Result<bool, Error> {
        self.seen.push("port-clear".to_owned());
        Ok(true)
    }

    fn claim(&mut self, _device: &DfuDevice, alt_setting: &str) -> Result<(), Error> {
        match self.event("claim")? {
            Event::Claim(alt) => {
                assert_eq!(alt, alt_setting);
                self.seen.push(format!("claim:{alt}"));
                Ok(())
            }
            event => panic!("expected claim event, got {event:?}"),
        }
    }

    fn release(&mut self) -> Result<(), Error> {
        match self.event("release")? {
            Event::Release => Ok(()),
            event => panic!("expected release event, got {event:?}"),
        }
    }

    fn transfer_size(&mut self) -> Result<usize, Error> {
        match self.event("transfer_size")? {
            Event::TransferSize(size) => Ok(size),
            event => panic!("expected transfer-size event, got {event:?}"),
        }
    }

    fn status(&mut self) -> Result<DfuStatus, Error> {
        match self.event("status")? {
            Event::Status(state, status) => Ok(DfuStatus {
                status,
                poll_timeout: Duration::ZERO,
                state,
            }),
            event => panic!("expected status event, got {event:?}"),
        }
    }

    fn clear_status(&mut self) -> Result<(), Error> {
        self.seen.push("clear".to_owned());
        Ok(())
    }

    fn abort(&mut self) -> Result<(), Error> {
        self.seen.push("abort".to_owned());
        Ok(())
    }

    fn download_chunk(&mut self, block: u16, data: &[u8]) -> Result<usize, Error> {
        match self.event("download")? {
            Event::Download {
                block: expected,
                bytes,
            } => {
                assert_eq!(block, expected);
                assert_eq!(data.len(), bytes);
                Ok(bytes)
            }
            event => panic!("expected download event, got {event:?}"),
        }
    }

    fn finish_download(&mut self, block: u16) -> Result<(), Error> {
        match self.event("finish")? {
            Event::Finish(expected) => {
                assert_eq!(block, expected);
                Ok(())
            }
            event => panic!("expected finish event, got {event:?}"),
        }
    }

    fn detach(&mut self, _timeout: Duration) -> Result<(), Error> {
        match self.event("detach")? {
            Event::Detach => {
                self.seen.push("detach".to_owned());
                Ok(())
            }
            event => panic!("expected detach event, got {event:?}"),
        }
    }

    fn reset(&mut self) -> Result<(), Error> {
        match self.event("reset")? {
            Event::Reset => {
                self.seen.push("reset".to_owned());
                Ok(())
            }
            event => panic!("expected reset event, got {event:?}"),
        }
    }

    fn sleep(&mut self, _duration: Duration) {}
}

fn device(path: UsbPath, alt: &str, address: u8) -> DfuDevice {
    DfuDevice {
        vendor_id: VID,
        product_id: PID,
        path,
        address,
        serial: Some(format!("serial-{address}")),
        alt_settings: vec![alt.to_owned()],
        name: "T3 test board".to_owned(),
    }
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn inputs(payloads: [&[u8]; 4]) -> Vec<DfuStageInput> {
    let names = ["tiboot3.bin", "tispl.bin", "u-boot.img", "rawemmc"];
    let alts = ["bootloader", "tispl.bin", "u-boot.img", "rawemmc"];
    payloads
        .into_iter()
        .enumerate()
        .map(|(index, payload)| DfuStageInput {
            stage: DfuStage {
                kind: if index < 3 {
                    DfuStageKind::BootArtifact {
                        next_alt_setting: alts[index + 1].to_owned(),
                    }
                } else {
                    DfuStageKind::RawEmmc
                },
                artifact_name: names[index].to_owned(),
                alt_setting: alts[index].to_owned(),
                reset_after: index < 3,
                reconnect_timeout: Duration::from_secs(15),
                expected_sha256: digest(payload),
                expected_size: Some(payload.len() as u64),
            },
            reader: Box::new(Cursor::new(payload.to_vec())),
        })
        .collect()
}

fn successful_events(payloads: [&[u8]; 4], terminal: DfuState) -> Vec<Event> {
    let alts = ["bootloader", "tispl.bin", "u-boot.img", "rawemmc"];
    let mut events = Vec::new();
    events.push(Event::Wait {
        alt: alts[0],
        address: 1,
    });
    for index in 0..4 {
        events.extend([
            Event::Claim(alts[index]),
            Event::Status(DfuState::DfuIdle, 0),
            Event::TransferSize(64),
            Event::Download {
                block: 0,
                bytes: payloads[index].len(),
            },
            Event::Status(DfuState::DfuDnloadIdle, 0),
            Event::Finish(1),
        ]);
        if index < 3 {
            // A boot artifact still has to be *manifested*: the zero-length download only asks to
            // end the transfer, and the device leaves dfuDNLOAD-SYNC when the host polls status.
            // A device that is still on the bus afterwards is waiting for the host to let go, so
            // it is detached and then reset — the pair is what makes U-Boot's DFU gadget leave its
            // download loop and boot what it was just given. The reset alone only restarts the
            // gadget, which is why the reference flow is `dfu-util -R` and not a bare reset.
            events.extend([
                Event::Status(DfuState::DfuManifest, 0),
                Event::Status(DfuState::DfuIdle, 0),
                Event::Detach,
                Event::Reset,
                Event::Release,
                Event::Wait {
                    alt: alts[index + 1],
                    address: (index + 2) as u8,
                },
            ]);
        } else {
            events.extend([
                Event::Status(DfuState::DfuManifest, 0),
                Event::Status(terminal, 0),
                Event::Detach,
                Event::Release,
            ]);
        }
    }
    events
}

/// Index of the status poll that opens the first boot artifact's manifestation.
fn first_boot_manifest_poll(events: &[Event]) -> usize {
    events
        .iter()
        .position(|event| matches!(event, Event::Status(DfuState::DfuManifest, _)))
        .expect("boot artifacts manifest")
}

/// dfuIDLE and dfuMANIFEST-WAIT-RESET are both "still on the bus", so both end in a reset: one
/// because DFU 1.1 says only a reset leaves that state, the other because a device that has not
/// left is a device still waiting to be let go.
#[test]
fn a_boot_artifact_that_parks_in_wait_reset_is_reset_like_any_other() {
    let payloads = [b"boot".as_slice(), b"spl", b"uboot", b"raw-image"];
    let mut events = successful_events(payloads, DfuState::DfuManifestWaitReset);
    let poll = first_boot_manifest_poll(&events);
    events[poll + 1] = Event::Status(DfuState::DfuManifestWaitReset, 0);

    let mut transport = MockTransport::new(events);
    let report = flash_with_transport(
        &mut transport,
        inputs(payloads),
        VID,
        PID,
        UsbPath::legacy(3, 7),
        None,
        None,
    )
    .unwrap();
    assert_eq!(report.stages.len(), 4);
    transport.done();
}

/// The TI ROM hands control to the artifact it just accepted, so the port drops before any final
/// status can be read. For a boot artifact that is the write succeeding, not the write failing —
/// and a board that has already left must not then be reset, because resetting the ROM restarts
/// its DFU loop and it comes back as `bootloader`/`SocId` instead of as the stage just written.
#[test]
fn a_boot_artifact_that_leaves_the_bus_while_manifesting_has_booted_and_is_not_reset() {
    let payloads = [b"boot".as_slice(), b"spl", b"uboot", b"raw-image"];
    let mut events = successful_events(payloads, DfuState::DfuManifestWaitReset);
    let poll = first_boot_manifest_poll(&events);
    events[poll] = Event::Fail("status", TransportErrorKind::Disconnected);
    // None of the second poll, the detach or the reset happens; the interface is still released.
    events.remove(poll + 1);
    events.remove(poll + 1);
    events.remove(poll + 1);

    let mut transport = MockTransport::new(events);
    let report = flash_with_transport(
        &mut transport,
        inputs(payloads),
        VID,
        PID,
        UsbPath::legacy(3, 7),
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        report.stages[0].terminal_evidence,
        DfuTerminalEvidence::NextAltEnumerated("tispl.bin".to_owned())
    );
    transport.done();
}

/// U-Boot's DFU gadget leaves its download loop from the detach trigger, which only `DFU_DETACH`
/// sets — a bare bus reset just restarts the gadget, so the board stays in DFU, never boots the
/// artifact, and the next stage silently writes into the instance that should have left. The
/// reference flow is `dfu-util -R`, which is DETACH *then* reset, so the order is asserted too.
#[test]
fn every_boot_stage_detaches_before_it_resets() {
    let payloads = [b"boot".as_slice(), b"spl", b"uboot", b"raw-image"];
    let mut transport = MockTransport::new(successful_events(payloads, DfuState::DfuIdle));
    flash_with_transport(
        &mut transport,
        inputs(payloads),
        VID,
        PID,
        UsbPath::legacy(3, 7),
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        transport
            .seen
            .iter()
            .filter(|event| *event == "detach" || *event == "reset")
            .cloned()
            .collect::<Vec<_>>(),
        // Three boot stages detach then reset; the raw stage only detaches.
        [
            "detach", "reset", "detach", "reset", "detach", "reset", "detach"
        ]
    );
    transport.done();
}

/// DFU 1.1 only defines `DFU_DETACH` for appIDLE, so a conformant device may stall it in dfuIDLE.
/// The reset that follows still drives such a device out, and `dfu-util -R` carries on the same
/// way — refusing here would fail writes the reference flow completes.
#[test]
fn a_boot_stage_detach_the_device_refuses_does_not_fail_the_write() {
    let payloads = [b"boot".as_slice(), b"spl", b"uboot", b"raw-image"];
    let mut events = successful_events(payloads, DfuState::DfuIdle);
    let first_detach = events
        .iter()
        .position(|event| matches!(event, Event::Detach))
        .expect("boot stages detach");
    events[first_detach] = Event::Fail("detach", TransportErrorKind::Pipe);

    let mut transport = MockTransport::new(events);
    let report = flash_with_transport(
        &mut transport,
        inputs(payloads),
        VID,
        PID,
        UsbPath::legacy(3, 7),
        None,
        None,
    )
    .unwrap();
    assert_eq!(report.stages.len(), 4);
    transport.done();
}

/// `libusb_reset_device` reports `LIBUSB_ERROR_NOT_FOUND` when the device had to be re-enumerated.
/// For a boot artifact that is the success case — the gadget left and the board comes back with the
/// next stage's alt-setting — so it must not end the write.
#[test]
fn a_boot_stage_reset_that_re_enumerates_the_device_does_not_fail_the_write() {
    let payloads = [b"boot".as_slice(), b"spl", b"uboot", b"raw-image"];
    let mut events = successful_events(payloads, DfuState::DfuIdle);
    let first_reset = events
        .iter()
        .position(|event| matches!(event, Event::Reset))
        .expect("boot stages reset");
    events[first_reset] = Event::Fail("reset", TransportErrorKind::NotFound);

    let mut transport = MockTransport::new(events);
    let report = flash_with_transport(
        &mut transport,
        inputs(payloads),
        VID,
        PID,
        UsbPath::legacy(3, 7),
        None,
        None,
    )
    .unwrap();
    assert_eq!(report.stages.len(), 4);
    transport.done();
}

#[test]
fn four_stages_run_in_exact_order_and_address_or_serial_changes_do_not_switch_ports() {
    let payloads = [b"boot".as_slice(), b"spl", b"uboot", b"raw-image"];
    let mut transport =
        MockTransport::new(successful_events(payloads, DfuState::DfuManifestWaitReset));
    let report = flash_with_transport(
        &mut transport,
        inputs(payloads),
        VID,
        PID,
        UsbPath::new(3, vec![2, 7]).unwrap(),
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        report
            .stages
            .iter()
            .map(|stage| stage.artifact_name.as_str())
            .collect::<Vec<_>>(),
        ["tiboot3.bin", "tispl.bin", "u-boot.img", "rawemmc"]
    );
    assert_eq!(
        report.stages[3].terminal_evidence,
        DfuTerminalEvidence::ManifestWaitReset
    );
    assert_eq!(
        transport
            .seen
            .iter()
            .filter(|event| event.starts_with("claim:"))
            .cloned()
            .collect::<Vec<_>>(),
        [
            "claim:bootloader",
            "claim:tispl.bin",
            "claim:u-boot.img",
            "claim:rawemmc"
        ]
    );
    transport.done();
}

#[test]
fn a_real_middle_stage_download_failure_is_not_swallowed() {
    let payloads = [b"boot".as_slice(), b"spl", b"uboot", b"raw"];
    let mut events = successful_events(payloads, DfuState::DfuIdle);
    let second_download = events
        .iter()
        .enumerate()
        .filter(|(_, event)| matches!(event, Event::Download { .. }))
        .nth(1)
        .unwrap()
        .0;
    events[second_download] = Event::Fail("download", TransportErrorKind::Access);
    // The state machine releases the still-connected failed interface and stops immediately.
    events.truncate(second_download + 1);
    events.push(Event::Release);
    let mut transport = MockTransport::new(events);
    let error = flash_with_transport(
        &mut transport,
        inputs(payloads),
        VID,
        PID,
        UsbPath::legacy(3, 7),
        None,
        None,
    )
    .unwrap_err();
    assert!(matches!(error, Error::StageTransfer { artifact, .. } if artifact == "tispl.bin"));
    transport.done();
}

#[test]
fn a_fatal_download_error_stops_at_each_of_the_four_stages() {
    let payloads = [b"boot".as_slice(), b"spl", b"uboot", b"raw"];
    let artifacts = ["tiboot3.bin", "tispl.bin", "u-boot.img", "rawemmc"];
    for (failing_stage, expected_artifact) in artifacts.iter().enumerate() {
        let mut events = successful_events(payloads, DfuState::DfuIdle);
        let target = events
            .iter()
            .enumerate()
            .filter(|(_, event)| matches!(event, Event::Download { .. }))
            .nth(failing_stage)
            .unwrap()
            .0;
        events[target] = Event::Fail("download", TransportErrorKind::Access);
        events.truncate(target + 1);
        events.push(Event::Release);
        let mut transport = MockTransport::new(events);
        let error = flash_with_transport(
            &mut transport,
            inputs(payloads),
            VID,
            PID,
            UsbPath::legacy(3, 7),
            None,
            None,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            Error::StageTransfer { artifact, .. } if artifact == *expected_artifact
        ));
        transport.done();
    }
}

#[test]
fn reset_disconnect_is_only_success_when_the_next_alt_enumerates() {
    let payloads = [b"boot".as_slice(), b"spl", b"uboot", b"raw"];
    let events = vec![
        Event::Wait {
            alt: "bootloader",
            address: 1,
        },
        Event::Claim("bootloader"),
        Event::Status(DfuState::DfuIdle, 0),
        Event::TransferSize(64),
        Event::Download { block: 0, bytes: 4 },
        Event::Fail("status", TransportErrorKind::Disconnected),
        Event::Wait {
            alt: "tispl.bin",
            address: 9,
        },
        Event::Claim("tispl.bin"),
        Event::Status(DfuState::DfuIdle, 0),
        Event::TransferSize(64),
        Event::Fail("download", TransportErrorKind::Access),
        Event::Release,
    ];
    let mut transport = MockTransport::new(events);
    let error = flash_with_transport(
        &mut transport,
        inputs(payloads),
        VID,
        PID,
        UsbPath::legacy(3, 7),
        None,
        None,
    )
    .unwrap_err();
    assert!(matches!(error, Error::StageTransfer { artifact, .. } if artifact == "tispl.bin"));
    transport.done();
}

#[test]
fn a_disconnect_reported_by_the_final_chunk_write_is_still_fatal() {
    let payloads = [b"boot".as_slice(), b"spl", b"uboot", b"raw"];
    let events = vec![
        Event::Wait {
            alt: "bootloader",
            address: 1,
        },
        Event::Claim("bootloader"),
        Event::Status(DfuState::DfuIdle, 0),
        Event::TransferSize(64),
        Event::Fail("download", TransportErrorKind::Disconnected),
        Event::Release,
    ];
    let mut transport = MockTransport::new(events);
    assert!(matches!(
        flash_with_transport(
            &mut transport,
            inputs(payloads),
            VID,
            PID,
            UsbPath::legacy(3, 7),
            None,
            None,
        ),
        Err(Error::StageTransfer { artifact, .. }) if artifact == "tiboot3.bin"
    ));
    transport.done();
}

#[test]
fn disconnect_without_the_expected_next_alt_times_out() {
    let payloads = [b"boot".as_slice(), b"spl", b"uboot", b"raw"];
    let events = vec![
        Event::Wait {
            alt: "bootloader",
            address: 1,
        },
        Event::Claim("bootloader"),
        Event::Status(DfuState::DfuIdle, 0),
        Event::TransferSize(64),
        Event::Download { block: 0, bytes: 4 },
        Event::Fail("status", TransportErrorKind::Disconnected),
        Event::Timeout("tispl.bin"),
    ];
    let mut transport = MockTransport::new(events);
    assert!(matches!(
        flash_with_transport(
            &mut transport,
            inputs(payloads),
            VID,
            PID,
            UsbPath::legacy(3, 7),
            None,
            None,
        ),
        Err(Error::ReconnectTimeout { alt_setting, .. }) if alt_setting == "tispl.bin"
    ));
    transport.done();
}

#[test]
fn zero_short_and_larger_than_u32_sources_fail_without_panicking() {
    let payloads = [b"boot".as_slice(), b"spl", b"uboot", b"raw"];
    let mut zero = inputs(payloads);
    zero[0].stage.expected_size = Some(0);
    let mut unused = MockTransport::new(vec![]);
    assert!(matches!(
        flash_with_transport(
            &mut unused,
            zero,
            VID,
            PID,
            UsbPath::legacy(1, 1),
            None,
            None,
        ),
        Err(Error::EmptyImage(_))
    ));

    let mut short = inputs(payloads);
    short[0].stage.expected_size = Some(10);
    let mut short_transport = MockTransport::new(vec![
        Event::Wait {
            alt: "bootloader",
            address: 1,
        },
        Event::Claim("bootloader"),
        Event::Status(DfuState::DfuIdle, 0),
        Event::TransferSize(64),
        Event::Download { block: 0, bytes: 4 },
        Event::Status(DfuState::DfuDnloadIdle, 0),
        Event::Release,
    ]);
    assert!(matches!(
        flash_with_transport(
            &mut short_transport,
            short,
            VID,
            PID,
            UsbPath::legacy(1, 1),
            None,
            None,
        ),
        Err(Error::ShortImage { .. })
    ));

    let mut huge = inputs(payloads);
    huge[0].stage.expected_size = Some(u64::from(u32::MAX) + 1);
    let mut huge_transport = MockTransport::new(vec![
        Event::Wait {
            alt: "bootloader",
            address: 1,
        },
        Event::Claim("bootloader"),
        Event::Status(DfuState::DfuIdle, 0),
        Event::TransferSize(64),
        Event::Fail("download", TransportErrorKind::Access),
        Event::Release,
    ]);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        flash_with_transport(
            &mut huge_transport,
            huge,
            VID,
            PID,
            UsbPath::legacy(1, 1),
            None,
            None,
        )
    }));
    assert!(matches!(result.unwrap(), Err(Error::StageTransfer { .. })));
}

#[test]
fn zlp_manifest_error_disconnect_and_final_detach_are_all_fatal() {
    let payloads = [b"boot".as_slice(), b"spl", b"uboot", b"raw"];

    for (replacement, expected) in [
        (Event::Fail("finish", TransportErrorKind::Access), "zlp"),
        (Event::Status(DfuState::DfuError, 7), "manifest"),
        (
            Event::Fail("status", TransportErrorKind::Disconnected),
            "disconnect",
        ),
        (Event::Fail("detach", TransportErrorKind::Access), "detach"),
    ] {
        let mut events = successful_events(payloads, DfuState::DfuIdle);
        let raw_claim = events
            .iter()
            .position(|event| matches!(event, Event::Claim("rawemmc")))
            .unwrap();
        let target = match expected {
            "zlp" => {
                events[raw_claim..]
                    .iter()
                    .position(|event| matches!(event, Event::Finish(_)))
                    .unwrap()
                    + raw_claim
            }
            "manifest" | "disconnect" => {
                events[raw_claim..]
                    .iter()
                    .position(|event| matches!(event, Event::Status(DfuState::DfuManifest, _)))
                    .unwrap()
                    + raw_claim
            }
            "detach" => {
                events[raw_claim..]
                    .iter()
                    .position(|event| matches!(event, Event::Detach))
                    .unwrap()
                    + raw_claim
            }
            _ => unreachable!(),
        };
        events[target] = replacement;
        events.truncate(target + 1);
        if expected == "zlp" {
            events.push(Event::Release);
        }
        let mut transport = MockTransport::new(events);
        let error = flash_with_transport(
            &mut transport,
            inputs(payloads),
            VID,
            PID,
            UsbPath::legacy(3, 7),
            None,
            None,
        )
        .unwrap_err();
        match expected {
            "zlp" => assert!(matches!(error, Error::Zlp { .. })),
            "manifest" => assert!(matches!(error, Error::UnexpectedState { .. })),
            "disconnect" => assert!(matches!(error, Error::DisconnectedBeforeTerminalState)),
            "detach" => assert!(matches!(error, Error::FinalDetach { .. })),
            _ => unreachable!(),
        }
        transport.done();
    }
}

#[test]
fn progress_names_every_measurable_and_indeterminate_phase_in_order() {
    let payloads = [b"boot".as_slice(), b"spl", b"uboot", b"raw"];
    let mut transport =
        MockTransport::new(successful_events(payloads, DfuState::DfuManifestWaitReset));
    let mut progress = Vec::new();
    let mut collect = |event| progress.push(event);

    flash_with_transport(
        &mut transport,
        inputs(payloads),
        VID,
        PID,
        UsbPath::legacy(3, 7),
        Some(&mut collect),
        None,
    )
    .unwrap();

    assert_eq!(
        progress,
        vec![
            DfuProgress::Reconnecting,
            DfuProgress::BootStage {
                index: 1,
                fraction: 1.0,
            },
            DfuProgress::Reconnecting,
            DfuProgress::BootStage {
                index: 2,
                fraction: 1.0,
            },
            DfuProgress::Reconnecting,
            DfuProgress::BootStage {
                index: 3,
                fraction: 1.0,
            },
            DfuProgress::Reconnecting,
            DfuProgress::RawWrite(1.0),
            DfuProgress::Finalizing,
        ]
    );
    transport.done();
}

struct AmbiguousTransport {
    path: UsbPath,
}

impl DfuTransport for AmbiguousTransport {
    fn enumerate(&mut self, _vendor_id: u16, _product_id: u16) -> Result<Vec<DfuDevice>, Error> {
        Ok(vec![
            device(self.path.clone(), "bootloader", 1),
            device(self.path.clone(), "bootloader", 2),
        ])
    }
    fn claim(&mut self, _: &DfuDevice, _: &str) -> Result<(), Error> {
        unreachable!()
    }
    fn release(&mut self) -> Result<(), Error> {
        unreachable!()
    }
    fn transfer_size(&mut self) -> Result<usize, Error> {
        unreachable!()
    }
    fn status(&mut self) -> Result<DfuStatus, Error> {
        unreachable!()
    }
    fn clear_status(&mut self) -> Result<(), Error> {
        unreachable!()
    }
    fn abort(&mut self) -> Result<(), Error> {
        unreachable!()
    }
    fn download_chunk(&mut self, _: u16, _: &[u8]) -> Result<usize, Error> {
        unreachable!()
    }
    fn finish_download(&mut self, _: u16) -> Result<(), Error> {
        unreachable!()
    }
    fn detach(&mut self, _: Duration) -> Result<(), Error> {
        unreachable!()
    }
    fn reset(&mut self) -> Result<(), Error> {
        unreachable!()
    }
    fn sleep(&mut self, _: Duration) {}
}

#[test]
fn two_devices_with_the_same_vid_pid_are_ambiguous() {
    let path = UsbPath::legacy(3, 7);
    let mut transport = AmbiguousTransport { path: path.clone() };
    let error = transport
        .wait_for_alt(
            VID,
            PID,
            &path,
            "bootloader",
            Instant::now() + Duration::from_secs(1),
            None,
        )
        .unwrap_err();
    assert!(matches!(error, Error::AmbiguousDevice { count: 2, .. }));
}

#[test]
fn cancellation_before_any_state_never_reports_success() {
    let token = CancellationToken::default();
    let guard = token.drop_guard();
    drop(guard);
    let payloads = [b"boot".as_slice(), b"spl", b"uboot", b"raw"];
    let mut transport = MockTransport::new(vec![]);
    assert!(matches!(
        flash_with_transport(
            &mut transport,
            inputs(payloads),
            VID,
            PID,
            UsbPath::legacy(3, 7),
            None,
            Some(&token),
        ),
        Err(Error::Aborted)
    ));
}
