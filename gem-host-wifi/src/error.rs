//! Errors that carry only *what failed*, never *the secret that failed to be read*.
//!
//! The plan (§12.3) is explicit: raw XML, D-Bus maps, Keychain queries and passphrases must not be
//! embedded in an error, because errors are logged, formatted and shown. Every variant here is a
//! small enum of causes plus, at most, an integer OS status code.

use thiserror::Error;

/// The operation that was in flight when a platform call failed. Lets an error name a stage without
/// quoting any of its data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    /// Enumerating network devices / connection profiles.
    ListDevices,
    /// Reading the active connection's SSID.
    ReadSsid,
    /// Resolving the saved profile that matches the active SSID (Windows).
    ResolveProfile,
    /// Reading the saved password / secrets.
    ReadSecret,
    /// Reading the regulatory or region-based country.
    ReadCountry,
}

/// A discovery or retrieval failure. Distinct from [`crate::PasswordOutcome`]: an outcome is a
/// normal, expected answer ("not stored", "open network"), whereas a `HostWifiError` means the
/// query itself could not be carried out.
#[derive(Error, Debug, PartialEq, Eq)]
pub enum HostWifiError {
    #[error("no Wi-Fi device is present on this host")]
    NoWifiDevice,

    #[error("no Wi-Fi network is currently connected")]
    NotConnected,

    #[error("the operating system denied access during {operation:?} without prompting")]
    PermissionDenied { operation: Operation },

    #[error("the {operation:?} platform call failed with status {code}")]
    PlatformApi { operation: Operation, code: i64 },

    #[error("more than one saved profile matched the active network; refusing to guess")]
    AmbiguousProfile,

    #[error("the connected SSID is not valid UTF-8")]
    InvalidSsidEncoding,

    #[error("the reported country code is not a valid two-letter code")]
    InvalidCountry,

    #[error("the saved profile could not be parsed")]
    MalformedProfile,

    #[error("host Wi-Fi discovery is not implemented for this platform")]
    UnsupportedPlatform,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_never_carries_data_only_operation_and_code() {
        // The Display strings name the operation and an integer, nothing secret-shaped.
        let e = HostWifiError::PlatformApi {
            operation: Operation::ReadSecret,
            code: 5,
        };
        let msg = e.to_string();
        assert!(msg.contains("ReadSecret"));
        assert!(msg.contains('5'));

        let denied = HostWifiError::PermissionDenied {
            operation: Operation::ReadSsid,
        };
        assert!(denied.to_string().contains("ReadSsid"));
    }
}
