pub const T3_DFU_HARDWARE_ID: &str = r"USB\VID_0451&PID_6165&REV_0200";

pub const T3_DFU_COMPATIBLE_ID: &str = r"USB\COMPAT_VID_0451&Class_FE&SubClass_01&Prot_02";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DriverState {
    #[default]
    NoDevice,
    ReadyWinUsb,
    ReadyExternal {
        service: String,
    },
    NeedsInstall,
    DriverConflict {
        service: Option<String>,
        problem_code: u32,
    },
    MultipleCandidates {
        count: usize,
    },
    ProbeFailed {
        win32_error: u32,
    },
    Unsupported,
}

impl DriverState {
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

pub(crate) fn classify(devices: &[DeviceFacts]) -> DriverState {
    if devices.is_empty() {
        return DriverState::NoDevice;
    }

    let driverless = devices
        .iter()
        .filter(|device| is_driverless(device))
        .count();

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
