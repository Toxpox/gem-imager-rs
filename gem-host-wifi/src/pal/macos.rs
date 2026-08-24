
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

type OSStatus = i32;

fn ensure_location_authorized() -> Result<(), HostWifiError> {
    use super::macos_location;

    macos_location::prime();
    match macos_location::wait_for_decision() {
        Some(status) if macos_location::is_authorized(status) => Ok(()),
        Some(status) => {
            tracing::info!(
                "Wi-Fi autofill blocked: CoreLocation reports {}",
                macos_location::describe(status)
            );
            Err(HostWifiError::PermissionDenied {
                operation: Operation::ReadSsid,
            })
        }
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
        | CWSecurity::DynamicWEP => SecurityKind::Enterprise,
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

        let ssid = match interface.ssid() {
            Some(ns) => DetectedSsid::Utf8(ns.to_string()),
            None => {
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

fn outcome_for_status(status: OSStatus) -> PasswordOutcome {
    match status {
        s if s == errSecItemNotFound => PasswordOutcome::NotStored,
        s if s == errSecUserCanceled => PasswordOutcome::UserCancelled,
        s if s == errSecAuthFailed => PasswordOutcome::UserDenied,
        s if s == errSecInteractionNotAllowed => PasswordOutcome::PermissionDenied,
        s if s == errSecMissingEntitlement => PasswordOutcome::PermissionDenied,
        _ => PasswordOutcome::Unavailable,
    }
}

pub(crate) fn read_saved_password(network: &NetworkRef) -> Result<PasswordOutcome, HostWifiError> {
    let NetworkRefInner::MacOs { ssid } = &network.0 else {
        return Err(HostWifiError::UnsupportedPlatform);
    };
    if ssid.is_empty() {
        return Ok(PasswordOutcome::NotStored);
    }

    read_system_keychain_password(ssid)
}

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

        // SAFETY: the pointer is null or a valid autoreleased NSString.
        let Some(password) = (unsafe { objc2::rc::Retained::retain(password) }) else {
            return Ok(PasswordOutcome::NotStored);
        };

        let mut text = password.to_string();
        let outcome = classify_credential(&text);
        // SAFETY: zeroes are valid UTF-8 and `text` is not read as text again.
        unsafe { text.as_bytes_mut() }.zeroize();
        Ok(outcome)
    })
}

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
        assert_ne!(
            outcome_for_status(errSecUserCanceled),
            outcome_for_status(errSecAuthFailed)
        );
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
        let empty = NetworkRef(NetworkRefInner::MacOs {
            ssid: String::new(),
        });
        assert_eq!(read_saved_password(&empty), Ok(PasswordOutcome::NotStored));
    }
}
