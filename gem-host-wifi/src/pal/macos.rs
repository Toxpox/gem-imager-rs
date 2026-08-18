//! macOS backend: CoreLocation gates access, CoreWLAN reads the network, Keychain holds the key.
//!
//! On macOS 14+ the SSID, BSSID and country code are all Location-gated (§6.1): the CoreWLAN
//! getters return `nil` unless Location Services is on and this app is authorized. If Location is
//! denied we do *not* fall back to a command-line workaround; we return
//! [`HostWifiError::PermissionDenied`] and the UI keeps the manual SSID field.
//!
//! Winning that authorization is its own problem, and it lives in [`super::macos_location`]: it
//! needs a delegate, a retained manager and the main run loop, none of which this module can
//! provide from the worker thread the detection runs on. Note that authorization is also refused
//! outright to a bundle that is not code-signed, so an unsigned build can never autofill.
//!
//! Password retrieval goes through CoreWLAN's `CWKeychainFindWiFiPassword` against the System
//! keychain domain (§6.4) — never the `security` CLI, never the Keychain database on disk, and never
//! an attempt to modify an item's ACL (§6.6). macOS stores a joined network's password in the System
//! keychain, owned by `airportd`, so a bare `SecItemCopyMatching` searches the login keychain, finds
//! nothing and never prompts.
//!
//! Every CoreWLAN/CoreLocation call is `unsafe` because it is an Objective-C message send; in each
//! case we only pass the receiver the framework gave us and immediately convert the result to an
//! owned Rust value.

use gem_helper::secret::Secret;
use objc2_core_wlan::{CWKeychainDomain, CWKeychainFindWiFiPassword, CWSecurity, CWWiFiClient};
use objc2_foundation::{NSData, NSString};
use objc2_security::{
    errSecAuthFailed, errSecInteractionNotAllowed, errSecItemNotFound, errSecMissingEntitlement,
    errSecSuccess, errSecUserCanceled,
};
use zeroize::Zeroize;

use crate::error::Operation;
use crate::model::CountryCode;
use crate::model::{
    CountryHint, CountrySource, DetectedSsid, DetectedWifi, NetworkRef, NetworkRefInner,
    SecurityKind,
};
use crate::{HostWifiError, PasswordOutcome};

/// The Security framework's result code. `objc2-security` keeps its own alias private, so it is
/// restated here against the same underlying type the `errSec*` constants are declared with.
type OSStatus = i32;

/// Ensure this process is authorized to read Location-gated Wi-Fi data, prompting once if the user
/// has not decided yet.
///
/// The prompt is asynchronous, so a first run that reaches this before the user has answered gets
/// [`HostWifiError::PermissionDenied`] and autofill simply happens on the next attempt (§6.1). What
/// this must never do is report "denied" merely because it asked too early — see
/// [`super::macos_location`] for why reading `authorizationStatus` directly does exactly that.
fn ensure_location_authorized() -> Result<(), HostWifiError> {
    use super::macos_location;

    macos_location::prime();
    match macos_location::wait_for_decision() {
        Some(status) if macos_location::is_authorized(status) => Ok(()),
        // Denied, restricted, or still undecided: no access this time round.
        Some(status) => {
            tracing::info!(
                "Wi-Fi autofill blocked: CoreLocation reports {}",
                macos_location::describe(status)
            );
            Err(HostWifiError::PermissionDenied {
                operation: Operation::ReadSsid,
            })
        }
        // Nothing came back at all. Either the main run loop never ran (so the request was never
        // dispatched), or macOS is ignoring an app it will not authorize — an unsigned bundle, or
        // one launched as a bare binary rather than from a .app.
        None => {
            tracing::warn!(
                "Wi-Fi autofill blocked: CoreLocation never reported an authorization status. \
                 This app must run from a code-signed .app bundle carrying \
                 NSLocationWhenInUseUsageDescription for macOS to grant Location access."
            );
            Err(HostWifiError::PermissionDenied {
                operation: Operation::ReadSsid,
            })
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
        // Dynamic WEP is 802.1X-based, so it belongs with the enterprise schemes.
        | CWSecurity::DynamicWEP => SecurityKind::Enterprise,
        // Static WEP is secured but carries no WPA-family passphrase the image can use. It must not
        // be reported as Open, which would tell the user no password is needed.
        CWSecurity::WEP => SecurityKind::UnsupportedSecurity,
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

/// Map a Keychain `OSStatus` to the outcome it actually describes (§6.4).
///
/// Constants come from the Security framework's `SecBase.h`. Every result here is a *normal*
/// answer, not a product failure: the manual password field stays the fallback.
fn outcome_for_status(status: OSStatus) -> PasswordOutcome {
    // Matched with guards rather than bare patterns: a lowercase constant used directly as a match
    // pattern would silently become a catch-all binding if the import ever broke.
    match status {
        // No AirPort item for this SSID: the network may have been joined on another device, or the
        // item may live in a keychain this app cannot see.
        s if s == errSecItemNotFound => PasswordOutcome::NotStored,
        s if s == errSecUserCanceled => PasswordOutcome::UserCancelled,
        // Deny, or a failed authentication. Kept distinct from Cancel: they mean different things.
        s if s == errSecAuthFailed => PasswordOutcome::UserDenied,
        // A prompt was required but could not be shown: a locked keychain or a non-interactive
        // session. Not a decision by the user, so it is reported separately.
        s if s == errSecInteractionNotAllowed => PasswordOutcome::PermissionDenied,
        // An unsigned or wrongly-entitled build (§6.3).
        s if s == errSecMissingEntitlement => PasswordOutcome::PermissionDenied,
        _ => PasswordOutcome::Unavailable,
    }
}

pub(crate) fn read_saved_password(network: &NetworkRef) -> Result<PasswordOutcome, HostWifiError> {
    let NetworkRefInner::MacOs { ssid } = &network.0 else {
        return Err(HostWifiError::UnsupportedPlatform);
    };
    // Discovery stores an empty SSID when the name was not representable as text; there is nothing
    // to look up in that case.
    if ssid.is_empty() {
        return Ok(PasswordOutcome::NotStored);
    }

    read_system_keychain_password(ssid)
}

/// Read the saved password for `ssid` from the System keychain, where macOS stores a joined
/// network's password. A bare `SecItemCopyMatching` would search the login keychain, find nothing
/// and never prompt; this is the call that raises the authorization dialog.
fn read_system_keychain_password(ssid: &str) -> Result<PasswordOutcome, HostWifiError> {
    objc2::rc::autoreleasepool(|_pool| {
        let ssid_data = NSData::with_bytes(ssid.as_bytes());
        let mut password: *mut NSString = std::ptr::null_mut();

        // SAFETY: `ssid_data` outlives the call and `password` is a valid out pointer the framework
        // either fills or leaves null.
        let status: OSStatus = unsafe {
            CWKeychainFindWiFiPassword(CWKeychainDomain::System, &ssid_data, &mut password)
        };

        if status != errSecSuccess {
            return Ok(outcome_for_status(status));
        }

        // The out parameter is `AutoreleasingUnsafeMutablePointer`, so the string comes back +0 and
        // must be retained rather than taken over with `from_raw`.
        // SAFETY: the pointer is null or a valid autoreleased NSString.
        let Some(password) = (unsafe { objc2::rc::Retained::retain(password) }) else {
            return Ok(PasswordOutcome::NotStored);
        };

        let mut text = password.to_string();
        let outcome = classify_credential(&text);
        // Wipe the plaintext before the buffer is freed (§12.2).
        // SAFETY: zeroes are valid UTF-8 and `text` is not read as text again.
        unsafe { text.as_bytes_mut() }.zeroize();
        Ok(outcome)
    })
}

/// Validate a credential read from the Keychain and wrap it, or say why it is unusable.
///
/// Split out so the rule is unit-testable without a Keychain (§3.1).
fn classify_credential(value: &str) -> PasswordOutcome {
    if !crate::is_usable_wifi_credential(value) {
        return PasswordOutcome::Unavailable;
    }
    PasswordOutcome::Found(Secret::new(value.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_classification_maps_personal_enterprise_and_open() {
        assert_eq!(
            classify_security(CWSecurity::WPA2Personal),
            SecurityKind::Personal
        );
        assert_eq!(
            classify_security(CWSecurity::WPA3Personal),
            SecurityKind::Personal
        );
        assert_eq!(
            classify_security(CWSecurity::WPA2Enterprise),
            SecurityKind::Enterprise
        );
        assert_eq!(classify_security(CWSecurity::None), SecurityKind::Open);
        assert_eq!(classify_security(CWSecurity::OWE), SecurityKind::Open);
    }

    #[test]
    fn static_wep_is_unsupported_and_dynamic_wep_is_enterprise() {
        // Static WEP is secured but carries nothing the image can use; calling it Open would tell
        // the user no password is needed for a secured network.
        assert_eq!(
            classify_security(CWSecurity::WEP),
            SecurityKind::UnsupportedSecurity
        );
        assert_eq!(
            classify_security(CWSecurity::DynamicWEP),
            SecurityKind::Enterprise
        );
    }

    #[test]
    fn keychain_statuses_map_to_the_documented_outcomes() {
        // Values from the Security framework's SecBase.h.
        assert_eq!(
            outcome_for_status(errSecItemNotFound),
            PasswordOutcome::NotStored
        );
        assert_eq!(
            outcome_for_status(errSecUserCanceled),
            PasswordOutcome::UserCancelled
        );
        assert_eq!(
            outcome_for_status(errSecAuthFailed),
            PasswordOutcome::UserDenied
        );
        assert_eq!(
            outcome_for_status(errSecInteractionNotAllowed),
            PasswordOutcome::PermissionDenied
        );
        assert_eq!(
            outcome_for_status(errSecMissingEntitlement),
            PasswordOutcome::PermissionDenied
        );
        // Cancel and Deny must stay distinguishable: they mean different things to the user.
        assert_ne!(
            outcome_for_status(errSecUserCanceled),
            outcome_for_status(errSecAuthFailed)
        );
        // Success is never routed through here, and an unmapped status is reported as unavailable
        // rather than being mistaken for a result.
        assert_eq!(outcome_for_status(-1), PasswordOutcome::Unavailable);
        assert_eq!(
            outcome_for_status(errSecSuccess),
            PasswordOutcome::Unavailable
        );
    }

    #[test]
    fn a_keychain_value_is_validated_before_it_is_handed_over() {
        assert_eq!(
            classify_credential("hunter2-pass"),
            PasswordOutcome::Found(Secret::new("hunter2-pass"))
        );
        let psk = "0DC0D6EB90555ED6419756B9A15EC3E3209B63DF707DD508D14581F8982721AF";
        assert_eq!(
            classify_credential(psk),
            PasswordOutcome::Found(Secret::new(psk))
        );
        // Too short, and a 64-byte value that is not hex, are both unusable.
        assert_eq!(classify_credential("short"), PasswordOutcome::Unavailable);
        assert_eq!(
            classify_credential(&"z".repeat(64)),
            PasswordOutcome::Unavailable
        );
        assert_eq!(classify_credential(""), PasswordOutcome::Unavailable);
    }

    #[test]
    fn a_network_ref_from_another_backend_is_refused() {
        let foreign = NetworkRef(NetworkRefInner::Opaque("elsewhere".to_owned()));
        assert_eq!(
            read_saved_password(&foreign),
            Err(HostWifiError::UnsupportedPlatform)
        );
    }

    #[test]
    fn an_unrepresentable_ssid_reports_nothing_stored_without_querying() {
        // Discovery leaves the SSID empty for a non-UTF-8 name; there is nothing to match on, and
        // no Keychain call should be attempted.
        let empty = NetworkRef(NetworkRefInner::MacOs {
            ssid: String::new(),
        });
        assert_eq!(read_saved_password(&empty), Ok(PasswordOutcome::NotStored));
    }
}
