#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod macos_location;
#[cfg(target_os = "windows")]
mod windows;

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

#[cfg(target_os = "macos")]
pub(crate) fn prime_location_authorization() {
    macos_location::prime();
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn prime_location_authorization() {}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub(crate) fn detect_current_wifi() -> Result<DetectedWifi, HostWifiError> {
    Err(HostWifiError::UnsupportedPlatform)
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub(crate) fn read_saved_password(_network: &NetworkRef) -> Result<PasswordOutcome, HostWifiError> {
    Err(HostWifiError::UnsupportedPlatform)
}
