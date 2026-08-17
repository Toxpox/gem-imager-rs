//! macOS backend: CoreLocation gates access, CoreWLAN reads the network, Keychain holds the key.
//!
//! On macOS 14+ the SSID, BSSID and country code are all Location-gated (§6.1): the CoreWLAN
//! getters return `nil` unless Location Services is on and this app is authorized. If Location is
//! denied we do *not* fall back to a command-line workaround; we return
//! [`HostWifiError::PermissionDenied`] and the UI keeps the manual SSID field.
//!
//! Password retrieval is a direct `SecItemCopyMatching` Keychain query (§6.4) — never the `security`
//! CLI, never the Keychain database on disk, and never an attempt to modify an item's ACL (§6.6).
//! The saved Wi-Fi password is the AirPort generic-password item whose account is the SSID. The
//! query runs in two passes: a metadata-only query establishes that exactly one item matches, and
//! only then does a second query ask for the bytes. That second call is the one that can prompt.
//!
//! Every CoreWLAN/CoreLocation call is `unsafe` because it is an Objective-C message send; in each
//! case we only pass the receiver the framework gave us and immediately convert the result to an
//! owned Rust value.

use std::ffi::c_void;
use std::ptr::NonNull;

use gem_helper::secret::Secret;
use objc2_core_foundation::{
    CFArray, CFBoolean, CFData, CFMutableDictionary, CFRetained, CFString, CFType, kCFBooleanTrue,
    kCFTypeDictionaryKeyCallBacks, kCFTypeDictionaryValueCallBacks,
};
use objc2_core_location::{CLAuthorizationStatus, CLLocationManager};
use objc2_core_wlan::{CWSecurity, CWWiFiClient};
use objc2_security::{
    SecItemCopyMatching, errSecAuthFailed, errSecInteractionNotAllowed, errSecItemNotFound,
    errSecMissingEntitlement, errSecSuccess, errSecUserCanceled, kSecAttrAccount,
    kSecAttrDescription, kSecClass, kSecClassGenericPassword, kSecMatchLimit, kSecMatchLimitAll,
    kSecMatchLimitOne, kSecReturnAttributes, kSecReturnData,
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
/// `notDetermined` triggers a request whose result arrives asynchronously via the delegate, so this
/// treats "just asked" like "not yet allowed" and the user re-runs the action once answered (§6.1).
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

/// Which of the two query stages to build (§6.4).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum QueryStage {
    /// Metadata only: how many items match, with no secret returned.
    CountOnly,
    /// The single match, returning its bytes. This is the call that can prompt.
    ReturnData,
}

/// `kCFBooleanTrue` as a raw pointer, for use as a query value.
fn true_value() -> *const c_void {
    // SAFETY: reading a CoreFoundation constant that the framework always defines. The binding is
    // an `Option` only because it cannot prove non-nullness at the type level.
    let boolean: &CFBoolean = unsafe { kCFBooleanTrue }.expect("kCFBooleanTrue is always defined");
    boolean as *const CFBoolean as *const c_void
}

/// Build the Keychain query for the AirPort password of exactly one SSID.
///
/// Apple's own network stack stores a saved Wi-Fi password as a generic-password item whose
/// `kSecAttrAccount` is the SSID and whose `kSecAttrDescription` is `"AirPort network password"`
/// (this is what CoreWLAN's `CWKeychainFindWiFiPassword` and `security -D "AirPort network
/// password" -a <ssid>` read). Apple does not document this as a stable retrieval contract (§6.4),
/// which is why a miss is [`PasswordOutcome::NotStored`] rather than a defect.
fn keychain_query(ssid: &str, stage: QueryStage) -> CFRetained<CFMutableDictionary> {
    // SAFETY: the dictionary is created empty and then filled with framework constants and
    // CoreFoundation objects created here; kCFTypeDictionary*CallBacks make it retain each one, so
    // nothing dangles once this function returns.
    unsafe {
        let query = CFMutableDictionary::new(
            None,
            5,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        )
        .expect("keychain query dictionary");

        let set = |key: &CFString, value: *const c_void| {
            CFMutableDictionary::set_value(
                Some(&query),
                key as *const CFString as *const c_void,
                value,
            );
        };

        set(
            kSecClass,
            kSecClassGenericPassword as *const CFString as *const c_void,
        );

        // Narrow to AirPort items so an unrelated generic password that merely shares the SSID as
        // its account name cannot be returned in its place.
        let description = CFString::from_str("AirPort network password");
        set(
            kSecAttrDescription,
            CFRetained::as_ptr(&description).as_ptr() as *const c_void,
        );

        // The SSID is the item's account, matched exactly: a prefix or case-insensitive match could
        // hand over the credential of a different network (§6.4).
        let account = CFString::from_str(ssid);
        set(
            kSecAttrAccount,
            CFRetained::as_ptr(&account).as_ptr() as *const c_void,
        );

        match stage {
            // Ask for *all* matches so an ambiguous result can be detected rather than silently
            // resolved to whichever item comes back first.
            QueryStage::CountOnly => {
                set(
                    kSecMatchLimit,
                    kSecMatchLimitAll as *const CFString as *const c_void,
                );
                set(kSecReturnAttributes, true_value());
            }
            QueryStage::ReturnData => {
                set(
                    kSecMatchLimit,
                    kSecMatchLimitOne as *const CFString as *const c_void,
                );
                set(kSecReturnData, true_value());
            }
        }

        query
    }
}

pub(crate) fn read_saved_password(network: &NetworkRef) -> Result<PasswordOutcome, HostWifiError> {
    let NetworkRefInner::MacOs { ssid } = &network.0 else {
        return Err(HostWifiError::UnsupportedPlatform);
    };
    // Discovery stores an empty SSID when the name was not representable as text; there is nothing
    // to match a Keychain item against in that case.
    if ssid.is_empty() {
        return Ok(PasswordOutcome::NotStored);
    }

    // Stage one: metadata only, establishing how many items match before anything asks for a
    // secret, so an ambiguous result is refused without ever reading a password (§6.4).
    match count_matching_items(ssid) {
        Ok(0) => return Ok(PasswordOutcome::NotStored),
        Ok(1) => {}
        Ok(_) => return Err(HostWifiError::AmbiguousProfile),
        Err(status) => return Ok(outcome_for_status(status)),
    }

    // Stage two: the single exact match, this time asking for the bytes. This is the call that can
    // prompt, which is why the public API is split into two explicit steps.
    read_single_item(ssid)
}

/// How many Keychain items match this SSID, without returning any secret data.
fn count_matching_items(ssid: &str) -> Result<usize, OSStatus> {
    let query = keychain_query(ssid, QueryStage::CountOnly);
    let mut result: *const CFType = std::ptr::null();

    // SAFETY: the query is a well-formed CFDictionary (a CFMutableDictionary is one) and `result`
    // is a valid out pointer that the framework either fills or leaves null.
    let status = unsafe { SecItemCopyMatching(&query, &mut result) };
    if status != errSecSuccess {
        return Err(status);
    }
    let Some(result) = NonNull::new(result.cast_mut()) else {
        return Ok(0);
    };

    // SAFETY: on success the framework returns a +1 object; `from_raw` takes that reference over so
    // it is released when `owned` drops.
    let owned = unsafe { CFRetained::from_raw(result) };
    // With kSecMatchLimitAll the result is an array. A lone dictionary (which some macOS versions
    // return for a single hit) means exactly one match.
    Ok(match owned.downcast_ref::<CFArray>() {
        Some(array) => array.count() as usize,
        None => 1,
    })
}

/// Read the one matching item's password bytes and move them straight into a [`Secret`].
fn read_single_item(ssid: &str) -> Result<PasswordOutcome, HostWifiError> {
    let query = keychain_query(ssid, QueryStage::ReturnData);
    let mut result: *const CFType = std::ptr::null();

    // SAFETY: as in `count_matching_items`; this variant asks for kSecReturnData, so a successful
    // result is a CFData rather than an array.
    let status = unsafe { SecItemCopyMatching(&query, &mut result) };
    if status != errSecSuccess {
        return Ok(outcome_for_status(status));
    }
    let Some(result) = NonNull::new(result.cast_mut()) else {
        return Ok(PasswordOutcome::NotStored);
    };

    // SAFETY: a +1 CFData we now own; its bytes are copied out and it is released on drop.
    let owned = unsafe { CFRetained::from_raw(result) };
    let Some(data) = owned.downcast_ref::<CFData>() else {
        return Ok(PasswordOutcome::Unavailable);
    };

    // The copied bytes are wiped before this returns, so an unusable credential never lingers in
    // memory (§12.2).
    let mut bytes = data.to_vec();
    let outcome = match std::str::from_utf8(&bytes) {
        Ok(text) => classify_credential(text),
        // A Wi-Fi passphrase is text; anything else is a Keychain item we cannot use.
        Err(_) => PasswordOutcome::Unavailable,
    };
    bytes.zeroize();
    Ok(outcome)
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

    #[test]
    fn the_two_query_stages_are_distinct() {
        // The metadata pass and the data pass must not be collapsed into one: the whole point of
        // the split is that nothing asks for a secret until exactly one item is known to match.
        assert_ne!(QueryStage::CountOnly, QueryStage::ReturnData);
    }
}
