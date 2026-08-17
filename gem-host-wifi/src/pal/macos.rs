//! macOS backend: CoreLocation gates access, CoreWLAN reads the network.
//!
//! On macOS 14+ the SSID, BSSID and country code are all Location-gated (research plan §6.1): the
//! CoreWLAN getters return `nil` unless Location Services is on and this app is authorized. So the
//! flow is:
//!
//! 1. Check [`CLLocationManager`] authorization; request it while in the foreground if undetermined.
//! 2. Get the default interface from the shared [`CWWiFiClient`].
//! 3. Read `ssid()`, `security()` and `countryCode()`.
//!
//! If Location is denied we do *not* fall back to a command-line workaround (§6.1); we return
//! [`HostWifiError::PermissionDenied`] and the UI keeps the manual SSID field.
//!
//! Password retrieval (a direct `SecItemCopyMatching` Keychain query, §6.4) is Faz 2. Until then
//! [`read_saved_password`] reports [`PasswordOutcome::Unavailable`].
//!
//! Every CoreWLAN/CoreLocation call is `unsafe` because it is an Objective-C message send; the
//! safety argument in each case is that we only ever pass the receiver the framework gave us and
//! immediately convert the result to an owned Rust value.

use objc2_core_location::{CLAuthorizationStatus, CLLocationManager};
use objc2_core_wlan::{CWSecurity, CWWiFiClient};

use crate::error::Operation;
use crate::model::{
    CountryHint, CountrySource, DetectedSsid, DetectedWifi, NetworkRef, NetworkRefInner,
    SecurityKind,
};
use crate::model::CountryCode;
use crate::{HostWifiError, PasswordOutcome};

/// Ensure this process is authorized to read Location-gated Wi-Fi data, prompting once if the user
/// has not decided yet.
///
/// Returns [`HostWifiError::PermissionDenied`] for the denied/restricted states so discovery can
/// stop cleanly. `notDetermined` triggers a request; the result of that request arrives
/// asynchronously via the delegate, so this call treats "just asked" like "not yet allowed" and the
/// user re-runs the action once the prompt is answered (§6.1).
fn ensure_location_authorized() -> Result<(), HostWifiError> {
    // SAFETY: `new`/`authorizationStatus`/`requestWhenInUseAuthorization` are standard instance
    // methods on CLLocationManager; we hold the manager alive for the duration of the call.
    unsafe {
        let manager = CLLocationManager::new();
        match manager.authorizationStatus() {
            CLAuthorizationStatus::AuthorizedWhenInUse
            | CLAuthorizationStatus::AuthorizedAlways => Ok(()),
            CLAuthorizationStatus::NotDetermined => {
                manager.requestWhenInUseAuthorization();
                // The grant is asynchronous; this run cannot yet read the SSID.
                Err(HostWifiError::PermissionDenied {
                    operation: Operation::ReadSsid,
                })
            }
            // Denied, restricted, or any future status: no access.
            _ => Err(HostWifiError::PermissionDenied {
                operation: Operation::ReadSsid,
            }),
        }
    }
}

/// Map CoreWLAN's fine-grained security enum onto our coarse [`SecurityKind`] (§8.4).
fn classify_security(security: CWSecurity) -> SecurityKind {
    match security {
        CWSecurity::None | CWSecurity::OWE | CWSecurity::OWETransition => SecurityKind::Open,
        CWSecurity::WPAPersonal
        | CWSecurity::WPAPersonalMixed
        | CWSecurity::WPA2Personal
        | CWSecurity::Personal
        | CWSecurity::WPA3Personal
        | CWSecurity::WPA3Transition => SecurityKind::Personal,
        CWSecurity::WPAEnterprise
        | CWSecurity::WPAEnterpriseMixed
        | CWSecurity::WPA2Enterprise
        | CWSecurity::Enterprise
        | CWSecurity::WPA3Enterprise
        // WEP and Dynamic WEP carry no single modern portable passphrase we support.
        | CWSecurity::WEP
        | CWSecurity::DynamicWEP => SecurityKind::Enterprise,
        _ => SecurityKind::Unknown,
    }
}

pub(crate) fn detect_current_wifi() -> Result<DetectedWifi, HostWifiError> {
    ensure_location_authorized()?;

    // SAFETY: the CoreWLAN client and interface are obtained from the framework singleton and used
    // only while retained here; each getter returns an autoreleased value we copy immediately.
    unsafe {
        let client = CWWiFiClient::sharedWiFiClient();
        let interface = client.interface().ok_or(HostWifiError::NoWifiDevice)?;

        // `ssid()` returns nil when not associated or when the SSID is not representable; a nil here
        // after authorization means "not connected".
        let ssid = match interface.ssid() {
            Some(ns) => DetectedSsid::Utf8(ns.to_string()),
            None => {
                // Distinguish "radio off / no network" from a genuinely non-UTF-8 SSID by falling
                // back to the raw bytes.
                match interface.ssidData() {
                    Some(data) => DetectedSsid::from_bytes(&data.to_vec()),
                    None => return Err(HostWifiError::NotConnected),
                }
            }
        };

        let security = classify_security(interface.security());

        let country = interface
            .countryCode()
            .and_then(|ns| CountryCode::parse(&ns.to_string()))
            .map(|code| CountryHint {
                code,
                // CoreWLAN's countryCode is the adopted regulatory code for the interface.
                source: CountrySource::Regulatory,
            })
            .or_else(crate::country_from_locale);

        let ssid_name = match &ssid {
            DetectedSsid::Utf8(name) => name.clone(),
            DetectedSsid::UnsupportedEncoding => String::new(),
        };

        Ok(DetectedWifi {
            network: NetworkRef(NetworkRefInner::MacOs { ssid: ssid_name }),
            ssid,
            security,
            country,
        })
    }
}

pub(crate) fn read_saved_password(
    _network: &NetworkRef,
) -> Result<PasswordOutcome, HostWifiError> {
    // Faz 2 issues a direct `SecItemCopyMatching` against the AirPort generic-password item for the
    // exact SSID (§6.4), which is the step that shows the Keychain prompt. Until then, do not guess.
    Ok(PasswordOutcome::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_classification_maps_personal_enterprise_and_open() {
        assert_eq!(classify_security(CWSecurity::WPA2Personal), SecurityKind::Personal);
        assert_eq!(classify_security(CWSecurity::WPA3Personal), SecurityKind::Personal);
        assert_eq!(classify_security(CWSecurity::WPA2Enterprise), SecurityKind::Enterprise);
        assert_eq!(classify_security(CWSecurity::None), SecurityKind::Open);
        assert_eq!(classify_security(CWSecurity::OWE), SecurityKind::Open);
    }
}
