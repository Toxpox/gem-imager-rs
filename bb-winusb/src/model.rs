/// Exact ROM identity that is allowed to trigger an installation offer.
pub const T3_DFU_HARDWARE_ID: &str = r"USB\VID_0451&PID_6165&REV_0200";

/// Exact compatible ID observed on the T3 ROM DFU devnode.
pub const T3_DFU_COMPATIBLE_ID: &str = r"USB\COMPAT_VID_0451&Class_FE&SubClass_01&Prot_02";

/// Canonical read-only state consumed by the GUI and re-checked by the elevated helper.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DriverState {
    /// No present devnode has both the exact ROM hardware ID and exact DFU compatible ID.
    #[default]
    NoDevice,
    /// The active function driver is WinUSB and the devnode has no PnP problem.
    ReadyWinUsb,
    /// A non-WinUSB service owns a healthy device. It is never replaced automatically.
    ReadyExternal { service: String },
    /// Exactly one exact ROM devnode has no bound driver/service. Windows reports either Code 28
    /// on a fresh host or problem code 0 after PnPUtil replaces an uninstalled package with the
    /// NULL driver.
    NeedsInstall,
    /// A present exact device has a driver or a problem that is not a safe driverless case.
    DriverConflict {
        service: Option<String>,
        problem_code: u32,
    },
    /// More than one exact ROM devnode is present, so hardware-ID-wide mutation is refused.
    MultipleCandidates { count: usize },
    /// The host probe itself failed. A failed probe must never become an install offer.
    ProbeFailed { win32_error: u32 },
    /// This platform has no Windows driver state.
    Unsupported,
}

impl DriverState {
    /// Only this state may enable the install action.
    pub const fn needs_install(&self) -> bool {
        matches!(self, Self::NeedsInstall)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeviceFacts {
    pub(crate) service: Option<String>,
    pub(crate) driver_key: Option<String>,
    pub(crate) problem_code: u32,
}

/// Keep policy separate from SetupAPI mechanics so every safety decision is unit-testable.
pub(crate) fn classify(devices: &[DeviceFacts]) -> DriverState {
    if devices.is_empty() {
        return DriverState::NoDevice;
    }

    let driverless = devices
        .iter()
        .filter(|device| is_driverless(device))
        .count();

    // libwdi's MVP installer binds by hardware ID, not by one SetupAPI instance. Even a healthy
    // second exact ROM devnode makes that mutation ambiguous, so refuse whenever more than one
    // exact present target exists (not merely when more than one of them is driverless).
    if devices.len() > 1 {
        return DriverState::MultipleCandidates {
            count: devices.len(),
        };
    }
    if driverless == 1 {
        return DriverState::NeedsInstall;
    }

    if devices.iter().all(|device| {
        device.problem_code == 0
            && device
                .service
                .as_deref()
                .is_some_and(|service| service.eq_ignore_ascii_case("WinUSB"))
    }) {
        return DriverState::ReadyWinUsb;
    }

    if devices.len() == 1 {
        let device = &devices[0];
        if device.problem_code == 0
            && let Some(service) = device
                .service
                .as_deref()
                .filter(|service| !service.is_empty())
        {
            return DriverState::ReadyExternal {
                service: service.to_owned(),
            };
        }
    }

    let device = &devices[0];
    DriverState::DriverConflict {
        service: device.service.clone().filter(|service| !service.is_empty()),
        problem_code: device.problem_code,
    }
}

fn is_driverless(device: &DeviceFacts) -> bool {
    // A clean Windows enumeration normally reports CM_PROB_FAILED_INSTALL (28). PnPUtil's
    // `/delete-driver ... /uninstall` path instead installs the NULL driver and can leave the
    // exact present devnode stopped with CM_PROB_NONE (0). Empty service *and* empty driver key
    // are the decisive facts in both cases; every other problem code remains non-mutating.
    matches!(device.problem_code, 0 | 28)
        && device.service.as_deref().is_none_or(str::is_empty)
        && device.driver_key.as_deref().is_none_or(str::is_empty)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(service: Option<&str>, driver_key: Option<&str>, problem_code: u32) -> DeviceFacts {
        DeviceFacts {
            service: service.map(str::to_owned),
            driver_key: driver_key.map(str::to_owned),
            problem_code,
        }
    }

    #[test]
    fn a_true_code_28_without_a_driver_needs_install() {
        assert_eq!(
            classify(&[facts(None, None, 28)]),
            DriverState::NeedsInstall
        );
        assert!(matches!(
            classify(&[facts(Some("usbccgp"), None, 28)]),
            DriverState::DriverConflict { .. }
        ));
        assert!(matches!(
            classify(&[facts(None, None, 10)]),
            DriverState::DriverConflict { .. }
        ));
    }

    #[test]
    fn pnputil_null_driver_with_problem_zero_needs_install() {
        assert_eq!(classify(&[facts(None, None, 0)]), DriverState::NeedsInstall);
        assert!(matches!(
            classify(&[facts(None, Some("{historical-driver-key}"), 0)]),
            DriverState::DriverConflict { .. }
        ));
    }

    #[test]
    fn zadig_winusb_is_ready_and_is_never_an_install_candidate() {
        assert_eq!(
            classify(&[facts(Some("WinUSB"), Some("{driver-key}"), 0)]),
            DriverState::ReadyWinUsb
        );
    }

    #[test]
    fn a_healthy_external_backend_is_preserved() {
        assert_eq!(
            classify(&[facts(Some("libusbK"), Some("{driver-key}"), 0)]),
            DriverState::ReadyExternal {
                service: "libusbK".into()
            }
        );
    }

    #[test]
    fn multiple_driverless_boards_never_pick_an_arbitrary_target() {
        assert_eq!(
            classify(&[facts(None, None, 28), facts(None, None, 28)]),
            DriverState::MultipleCandidates { count: 2 }
        );
    }

    #[test]
    fn one_ready_and_one_driverless_board_is_still_ambiguous() {
        assert_eq!(
            classify(&[
                facts(None, None, 28),
                facts(Some("WinUSB"), Some("{driver-key}"), 0),
            ]),
            DriverState::MultipleCandidates { count: 2 }
        );
    }
}
