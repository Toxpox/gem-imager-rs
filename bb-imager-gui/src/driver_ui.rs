//! UI state machine for the Windows DFU driver offer.
//!
//! It intentionally contains no PnP mutation. The GUI can only probe and launch the fixed helper;
//! the elevated helper repeats the trust decision before it changes the machine.

use std::time::Duration;

use bb_winusb::{DriverState, InstallError, InstallOutcome};
use iced::{Subscription, Task};
use tokio::time::interval;

use crate::{helpers::blocking_future, message::BBImagerMessage};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum DriverPanel {
    #[default]
    Hidden,
    Offer,
    Details,
    Installing,
    Ready,
    Failed(String),
}

#[derive(Debug, Default)]
pub(crate) struct DfuDriverUiState {
    last_probe: DriverState,
    panel: DriverPanel,
    dismissed_until_detach: bool,
}

impl DfuDriverUiState {
    pub(crate) fn panel(&self) -> &DriverPanel {
        &self.panel
    }

    pub(crate) fn on_probe(&mut self, state: DriverState) {
        if state == self.last_probe {
            return;
        }
        tracing::info!(previous = ?self.last_probe, current = ?state, "T3 ROM DFU driver state changed");

        match &state {
            DriverState::NoDevice => {
                self.dismissed_until_detach = false;
                if !matches!(self.panel, DriverPanel::Installing) {
                    self.panel = DriverPanel::Hidden;
                }
            }
            DriverState::NeedsInstall
                if !self.dismissed_until_detach && matches!(self.panel, DriverPanel::Hidden) =>
            {
                self.panel = DriverPanel::Offer;
            }
            DriverState::ReadyWinUsb | DriverState::ReadyExternal { .. } => {
                if matches!(
                    self.panel,
                    DriverPanel::Offer | DriverPanel::Details | DriverPanel::Failed(_)
                ) {
                    self.panel = DriverPanel::Hidden;
                }
            }
            DriverState::DriverConflict { .. }
            | DriverState::MultipleCandidates { .. }
            | DriverState::ProbeFailed { .. }
            | DriverState::Unsupported => {
                if matches!(self.panel, DriverPanel::Offer | DriverPanel::Details) {
                    self.panel = DriverPanel::Hidden;
                }
            }
            _ => {}
        }
        self.last_probe = state;
    }

    pub(crate) fn dismiss(&mut self) {
        if matches!(
            self.panel,
            DriverPanel::Offer | DriverPanel::Details | DriverPanel::Failed(_)
        ) {
            self.dismissed_until_detach = true;
        }
        self.panel = DriverPanel::Hidden;
    }

    pub(crate) fn show_details(&mut self) {
        if matches!(self.panel, DriverPanel::Offer | DriverPanel::Failed(_)) {
            self.panel = DriverPanel::Details;
        }
    }

    pub(crate) fn back_to_offer(&mut self) {
        if matches!(self.panel, DriverPanel::Details) {
            self.panel = DriverPanel::Offer;
        }
    }

    pub(crate) fn begin_install(&mut self) -> bool {
        if self.last_probe.needs_install()
            && matches!(self.panel, DriverPanel::Offer | DriverPanel::Failed(_))
        {
            self.panel = DriverPanel::Installing;
            true
        } else {
            false
        }
    }

    pub(crate) fn finish_install(&mut self, result: Result<InstallOutcome, InstallError>) {
        self.panel = match result {
            Ok(InstallOutcome::Installed | InstallOutcome::AlreadyReady) => DriverPanel::Ready,
            Err(error) => DriverPanel::Failed(error.to_string()),
        };
    }
}

pub(crate) fn subscription() -> Subscription<BBImagerMessage> {
    Subscription::run_with((), |_| {
        let mut ticks = interval(Duration::from_secs(2));
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        iced::futures::stream::unfold(ticks, async move |mut ticks| {
            // Tokio's first interval tick completes immediately, covering a board attached before
            // application startup. Subsequent ticks also cover hotplug without trusting an event
            // payload as an install target.
            ticks.tick().await;
            let state = blocking_future(bb_winusb::probe).await;
            Some((BBImagerMessage::DfuDriverProbe(state), ticks))
        })
    })
}

pub(crate) fn install_task() -> Task<BBImagerMessage> {
    Task::perform(
        blocking_future(|| bb_winusb::launch_elevated_helper(env!("BB_WINUSB_HELPER_SHA256"))),
        BBImagerMessage::DfuDriverInstallFinished,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_offer_per_attachment_session() {
        let mut state = DfuDriverUiState::default();
        state.on_probe(DriverState::NeedsInstall);
        assert_eq!(state.panel(), &DriverPanel::Offer);

        state.dismiss();
        state.on_probe(DriverState::NeedsInstall);
        assert_eq!(state.panel(), &DriverPanel::Hidden);

        state.on_probe(DriverState::NoDevice);
        state.on_probe(DriverState::NeedsInstall);
        assert_eq!(state.panel(), &DriverPanel::Offer);
    }

    #[test]
    fn ready_zadig_state_never_opens_the_offer() {
        let mut state = DfuDriverUiState::default();
        state.on_probe(DriverState::ReadyWinUsb);
        assert_eq!(state.panel(), &DriverPanel::Hidden);
    }

    #[test]
    fn install_cannot_start_after_the_device_leaves_the_driverless_state() {
        let mut state = DfuDriverUiState::default();
        state.on_probe(DriverState::NeedsInstall);
        state.on_probe(DriverState::DriverConflict {
            service: Some("usbccgp".into()),
            problem_code: 10,
        });
        assert_eq!(state.panel(), &DriverPanel::Hidden);
        assert!(!state.begin_install());
    }
}
