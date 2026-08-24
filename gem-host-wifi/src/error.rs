
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    ListDevices,
    ReadSsid,
    ResolveProfile,
    ReadSecret,
    ReadCountry,
}

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
