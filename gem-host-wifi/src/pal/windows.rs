//! Windows backend: WinRT for the connected SSID and home region.
//!
//! The connected SSID comes from the WinRT connection profile, not `WlanQueryInterface`
//! (research plan §7.1): since the 2024 Windows 11 Wi-Fi location-privacy changes,
//! `WlanQueryInterface(current_connection)` can return `ERROR_ACCESS_DENIED` when location is off,
//! whereas `WlanConnectionProfileDetails.GetConnectedSsid()` keeps working. The chain is:
//!
//! ```text
//! NetworkInformation::GetConnectionProfiles()
//!   -> the profile with IsWlanConnectionProfile() == true
//!      -> WlanConnectionProfileDetails().GetConnectedSsid()   (the SSID)
//!      -> NetworkAdapter().NetworkAdapterId()                 (interface GUID, kept for Faz 2)
//! ```
//!
//! The country is a best-effort guess from `GlobalizationPreferences::HomeGeographicRegion`
//! (§7.6): Windows has no unprivileged API for the *radio's* regulatory country, so this is tagged
//! [`CountrySource::UserRegion`] and the UI leaves it editable.
//!
//! Security classification and the saved password both need the Win32 native profile, whose XML is
//! parsed in Faz 2. Discovery therefore reports [`SecurityKind::Unknown`]; it never guesses.

use windows::Networking::Connectivity::NetworkInformation;
use windows::System::UserProfile::GlobalizationPreferences;

use crate::error::Operation;
use crate::model::{
    CountryCode, CountryHint, CountrySource, DetectedSsid, DetectedWifi, NetworkRef,
    NetworkRefInner, SecurityKind,
};
use crate::{HostWifiError, PasswordOutcome};

/// Turn a WinRT `windows::core::Error` into our data-free error for a given stage.
fn platform_err(operation: Operation) -> impl Fn(windows::core::Error) -> HostWifiError {
    move |e| {
        // HRESULT for E_ACCESSDENIED / access-denied family; treat as a permission problem so the
        // UI can say "location is off" rather than a generic platform failure.
        const E_ACCESSDENIED: i32 = -0x7FFF_BFFB; // 0x80070005 as i32
        if e.code().0 == E_ACCESSDENIED {
            HostWifiError::PermissionDenied { operation }
        } else {
            HostWifiError::PlatformApi {
                operation,
                code: e.code().0 as i64,
            }
        }
    }
}

pub(crate) fn detect_current_wifi() -> Result<DetectedWifi, HostWifiError> {
    let profiles = NetworkInformation::GetConnectionProfiles()
        .map_err(platform_err(Operation::ListDevices))?;

    let mut saw_wlan = false;
    for profile in profiles {
        let is_wlan = profile
            .IsWlanConnectionProfile()
            .map_err(platform_err(Operation::ListDevices))?;
        if !is_wlan {
            continue;
        }
        saw_wlan = true;

        let details = profile
            .WlanConnectionProfileDetails()
            .map_err(platform_err(Operation::ReadSsid))?;
        let ssid_hstring = details
            .GetConnectedSsid()
            .map_err(platform_err(Operation::ReadSsid))?;
        let ssid_text = ssid_hstring.to_string();
        // An empty SSID here means the WLAN profile exists but is not currently associated.
        if ssid_text.is_empty() {
            continue;
        }

        // The interface GUID is not needed for discovery, but Faz 2's WlanGetProfile lookup keys off
        // it. Capture it now so the returned NetworkRef is self-contained.
        let interface_guid = profile
            .NetworkAdapter()
            .and_then(|na| na.NetworkAdapterId())
            .map(|guid| format!("{guid:?}"))
            .unwrap_or_default();

        // WinRT hands back the SSID as text; represent it through the same byte-classifying path so
        // the model stays consistent, even though HSTRING is already UTF-16-derived.
        let ssid = DetectedSsid::from_bytes(ssid_text.as_bytes());

        return Ok(DetectedWifi {
            network: NetworkRef(NetworkRefInner::Windows {
                interface_guid,
                // Faz 2 resolves the real native profile name from the SSID; store the SSID as the
                // starting point rather than assuming the profile name equals it (§7.2).
                profile_name: ssid_text,
            }),
            ssid,
            // Security type comes from the Win32 profile XML in Faz 2; do not guess here.
            security: SecurityKind::Unknown,
            country: detect_country(),
        });
    }

    Err(if saw_wlan {
        HostWifiError::NotConnected
    } else {
        HostWifiError::NoWifiDevice
    })
}

/// Best-effort country from the user's home region (§7.6), tagged as a non-authoritative guess.
/// Falls back to the process locale when the region is missing or not a valid code.
fn detect_country() -> Option<CountryHint> {
    let from_region = GlobalizationPreferences::HomeGeographicRegion()
        .ok()
        .and_then(|region| CountryCode::parse(&region.to_string()))
        .map(|code| CountryHint {
            code,
            source: CountrySource::UserRegion,
        });
    from_region.or_else(crate::country_from_locale)
}

pub(crate) fn read_saved_password(
    _network: &NetworkRef,
) -> Result<PasswordOutcome, HostWifiError> {
    // Faz 2 resolves the native profile for this SSID and calls WlanGetProfile with
    // WLAN_PROFILE_GET_PLAINTEXT_KEY, then validates the returned XML (§7.3). Until then, do not
    // guess: the UI keeps the manual password field.
    Ok(PasswordOutcome::Unavailable)
}

