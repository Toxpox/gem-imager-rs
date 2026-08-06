//! Windows-only readiness and provisioning boundary for the T3 AM62x ROM DFU device.
//!
//! The read-only probe deliberately uses Windows PnP state instead of libusb enumeration. A
//! driverless USB device is exactly the case in which libusb cannot be trusted to see enough to
//! decide that changing system state is safe. The mutating operation lives in the separately
//! elevated `bb-winusb-helper` process and re-runs the same probe before calling libwdi.

mod model;

#[cfg(windows)]
mod install;
#[cfg(windows)]
mod setupapi;

pub use model::{DriverState, T3_DFU_COMPATIBLE_ID, T3_DFU_HARDWARE_ID};

/// The model ID intentionally matches the package Zadig generated during hardware validation.
/// It covers later DFU stages whose `REV_` value differs from the ROM's `0200` value.
pub const T3_DFU_PACKAGE_ID: &str = r"USB\VID_0451&PID_6165";

/// Stable interface GUID emitted into every Gem Imager-generated WinUSB INF.
pub const T3_DFU_DEVICE_INTERFACE_GUID: &str = "{5F6D4A65-2E0F-4F1C-9C7A-7E6130F6F651}";

/// Read the current Windows PnP state without changing the machine.
#[cfg(windows)]
pub fn probe() -> DriverState {
    setupapi::probe()
}

/// Non-Windows builds retain a small stub so the feature remains cross-compilable.
#[cfg(not(windows))]
pub const fn probe() -> DriverState {
    DriverState::Unsupported
}

#[cfg(windows)]
pub use install::{
    HELPER_EXIT_ALREADY_READY, HELPER_EXIT_INSTALLED, HELPER_EXIT_INVALID_COMMAND, InstallError,
    InstallOutcome, install_t3_rom_dfu, launch_elevated_helper, verify_dfu_transport,
};

#[cfg(not(windows))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallError(pub String);

#[cfg(not(windows))]
impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(not(windows))]
impl std::error::Error for InstallError {}

#[cfg(not(windows))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallOutcome {
    Installed,
    AlreadyReady,
}

#[cfg(not(windows))]
pub fn install_t3_rom_dfu() -> Result<InstallOutcome, InstallError> {
    Err(InstallError(
        "WinUSB provisioning is only available on Windows".into(),
    ))
}

#[cfg(not(windows))]
pub fn launch_elevated_helper(
    _expected_helper_sha256: &str,
) -> Result<InstallOutcome, InstallError> {
    Err(InstallError(
        "WinUSB provisioning is only available on Windows".into(),
    ))
}

#[cfg(not(windows))]
pub fn verify_dfu_transport() -> Result<(), String> {
    Err("DFU WinUSB transport verification is only available on Windows".into())
}
