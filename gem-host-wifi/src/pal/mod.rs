//! Platform dispatch. Each `cfg` arm forwards to a backend; platforms without one report
//! [`HostWifiError::UnsupportedPlatform`] so a caller on an unsupported OS degrades to manual entry
//! instead of failing to build.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

// The Windows profile XML parser holds no Windows types, so it is also compiled under `test` on
// every platform: that way its fixture matrix (§7.5) runs in the ordinary test suite instead of
// only on a Windows host.
#[cfg(any(target_os = "windows", test))]
pub(crate) mod wlan_profile;

use crate::{DetectedWifi, HostWifiError, NetworkRef, PasswordOutcome};

#[cfg(target_os = "linux")]
pub(crate) fn detect_current_wifi() -> Result<DetectedWifi, HostWifiError> {
    linux::detect_current_wifi()
}

#[cfg(target_os = "linux")]
pub(crate) fn read_saved_password(network: &NetworkRef) -> Result<PasswordOutcome, HostWifiError> {
    linux::read_saved_password(network)
}

#[cfg(target_os = "windows")]
pub(crate) fn detect_current_wifi() -> Result<DetectedWifi, HostWifiError> {
    windows::detect_current_wifi()
}

#[cfg(target_os = "windows")]
pub(crate) fn read_saved_password(network: &NetworkRef) -> Result<PasswordOutcome, HostWifiError> {
    windows::read_saved_password(network)
}

#[cfg(target_os = "macos")]
pub(crate) fn detect_current_wifi() -> Result<DetectedWifi, HostWifiError> {
    macos::detect_current_wifi()
}

#[cfg(target_os = "macos")]
pub(crate) fn read_saved_password(network: &NetworkRef) -> Result<PasswordOutcome, HostWifiError> {
    macos::read_saved_password(network)
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub(crate) fn detect_current_wifi() -> Result<DetectedWifi, HostWifiError> {
    Err(HostWifiError::UnsupportedPlatform)
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub(crate) fn read_saved_password(_network: &NetworkRef) -> Result<PasswordOutcome, HostWifiError> {
    Err(HostWifiError::UnsupportedPlatform)
}
