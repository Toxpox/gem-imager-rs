//! Windows backend: WinRT for the connected SSID and home region.
//!
//! The connected SSID comes from the WinRT connection profile, not `WlanQueryInterface` (§7.1):
//! since the 2024 Windows 11 Wi-Fi location-privacy changes that call can return
//! `ERROR_ACCESS_DENIED` when location is off, whereas `GetConnectedSsid()` keeps working.
//!
//! ```text
//! NetworkInformation::GetConnectionProfiles()
//!   -> the profile with IsWlanConnectionProfile() == true
//!      -> WlanConnectionProfileDetails().GetConnectedSsid()   (the SSID)
//!      -> NetworkAdapter().NetworkAdapterId()                 (the interface GUID)
//! ```
//!
//! The country is a best-effort guess from `GlobalizationPreferences::HomeGeographicRegion` (§7.6):
//! Windows has no unprivileged API for the *radio's* regulatory country, so it is tagged
//! [`CountrySource::UserRegion`] and left editable.
//!
//! The security type and the saved password both come from the Win32 native profile, resolved by
//! matching its SSID against the connected one (§7.2) and parsed in [`crate::pal::wlan_profile`].
//! Discovery reads only the non-secret part of that document; the plaintext flag is used solely by
//! [`read_saved_password`].

use gem_helper::secret::{DerivedSecret, Secret};
use windows::Networking::Connectivity::NetworkInformation;
use windows::System::UserProfile::GlobalizationPreferences;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::NetworkManagement::WiFi::{
    WLAN_PROFILE_GET_PLAINTEXT_KEY, WLAN_PROFILE_INFO_LIST, WlanCloseHandle, WlanFreeMemory,
    WlanGetProfile, WlanGetProfileList, WlanOpenHandle,
};
use windows::core::{GUID, HSTRING, PWSTR};

use crate::error::Operation;
use crate::model::{
    CountryCode, CountryHint, CountrySource, DetectedSsid, DetectedWifi, NetworkRef,
    NetworkRefInner, SecurityKind,
};
use crate::pal::wlan_profile;
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

        // The interface GUID is not needed for the SSID itself, but the WlanGetProfile lookup keys
        // off it. Capture it now so the returned NetworkRef is self-contained.
        let interface_guid = profile
            .NetworkAdapter()
            .and_then(|na| na.NetworkAdapterId())
            .map(|guid| format!("{guid:?}"))
            .unwrap_or_default();

        // WinRT hands back the SSID as text; represent it through the same byte-classifying path so
        // the model stays consistent, even though HSTRING is already UTF-16-derived.
        let ssid = DetectedSsid::from_bytes(ssid_text.as_bytes());

        // The security type lives in the native profile, so classify it from there — reading each
        // candidate *without* the plaintext flag, so discovery never touches a key. A failure here
        // is not fatal: the SSID and country are still worth filling in.
        let security =
            detect_security(&interface_guid, &ssid_text).unwrap_or(SecurityKind::Unknown);

        return Ok(DetectedWifi {
            network: NetworkRef(NetworkRefInner::Windows {
                interface_guid,
                // Retrieval resolves the real native profile name from the SSID; store the SSID as
                // the starting point rather than assuming the profile name equals it (§7.2).
                profile_name: ssid_text,
            }),
            ssid,
            security,
            country: detect_country(),
        });
    }

    Err(if saw_wlan {
        HostWifiError::NotConnected
    } else {
        HostWifiError::NoWifiDevice
    })
}

/// Classify the connected network's security from its saved profile, without reading any key.
fn detect_security(interface_guid: &str, ssid: &str) -> Option<SecurityKind> {
    let interface = parse_interface_guid(interface_guid).ok()?;
    let handle = WlanHandle::open().ok()?;
    let profile_name = resolve_profile(&handle, &interface, ssid).ok()?;
    // `plaintext = false`: the security type is in the non-secret part of the document.
    let xml = get_profile_xml(&handle, &interface, &profile_name, false).ok()?;
    Some(wlan_profile::parse(&xml)?.security)
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

/// A `WlanOpenHandle` handle that closes itself, so no early return can leak it.
struct WlanHandle(HANDLE);

impl WlanHandle {
    fn open() -> Result<Self, HostWifiError> {
        let mut negotiated = 0u32;
        let mut handle = HANDLE::default();
        // Client version 2 is the Vista-and-later Native Wi-Fi API.
        let status = unsafe { WlanOpenHandle(2, None, &mut negotiated, &mut handle) };
        win32_result(status, Operation::ReadSecret)?;
        Ok(Self(handle))
    }
}

impl Drop for WlanHandle {
    fn drop(&mut self) {
        unsafe {
            WlanCloseHandle(self.0, None);
        }
    }
}

/// A pointer owned by the WLAN API, freed with `WlanFreeMemory` however the caller leaves scope.
struct WlanMemory<T>(*mut T);

impl<T> Drop for WlanMemory<T> {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WlanFreeMemory(self.0 as *const core::ffi::c_void) };
        }
    }
}

/// Turn a Win32 `DWORD` status into our error type. `ERROR_SUCCESS` is 0.
fn win32_result(status: u32, operation: Operation) -> Result<(), HostWifiError> {
    match status {
        0 => Ok(()),
        // ERROR_ACCESS_DENIED. Expected without elevation, or with a restrictive WLAN DACL (§7.4).
        5 => Err(HostWifiError::PermissionDenied { operation }),
        code => Err(HostWifiError::PlatformApi {
            operation,
            code: code as i64,
        }),
    }
}

/// Whether a Win32 status means "the secret specifically is not available", as opposed to a failure
/// of the query itself.
///
/// `ERROR_ACCESS_DENIED` here is a normal outcome: on a debug build running as `asInvoker`, or under
/// a restrictive DACL, the plaintext key is simply not granted (§7.4).
fn secret_outcome_for_status(status: u32) -> Option<PasswordOutcome> {
    match status {
        5 => Some(PasswordOutcome::PermissionDenied),
        // ERROR_NOT_FOUND / ERROR_NO_MATCH: the profile vanished between listing and reading.
        1168 | 1169 => Some(PasswordOutcome::NotStored),
        _ => None,
    }
}

/// Parse the interface GUID that discovery stored in the [`NetworkRef`].
///
/// Discovery formats it with `Debug`, which for `windows_core::GUID` is the standard braced form
/// that `GUID::try_from(&str)` accepts.
fn parse_interface_guid(text: &str) -> Result<GUID, HostWifiError> {
    GUID::try_from(text).map_err(|_| HostWifiError::MalformedProfile)
}

/// Read one profile's XML, asking for the plaintext key when `plaintext` is set.
///
/// The returned buffer belongs to the WLAN API. It is copied into a zeroizing string and the native
/// buffer is overwritten before being freed (§7.5).
fn get_profile_xml(
    handle: &WlanHandle,
    interface: &GUID,
    profile_name: &str,
    plaintext: bool,
) -> Result<DerivedSecret, ProfileReadError> {
    let name: HSTRING = HSTRING::from(profile_name);
    let mut xml: PWSTR = PWSTR::null();
    let mut flags: u32 = if plaintext {
        WLAN_PROFILE_GET_PLAINTEXT_KEY
    } else {
        0
    };
    let mut access: u32 = 0;

    let status = unsafe {
        WlanGetProfile(
            handle.0,
            interface,
            &name,
            None,
            &mut xml,
            Some(&mut flags),
            Some(&mut access),
        )
    };
    if status != 0 {
        return Err(ProfileReadError::Status(status));
    }
    if xml.is_null() {
        return Err(ProfileReadError::Status(0));
    }

    // SAFETY: on success WlanGetProfile hands back a NUL-terminated UTF-16 buffer it owns.
    let text = unsafe {
        let len = xml.len();
        let slice = std::slice::from_raw_parts(xml.0, len);
        let owned = DerivedSecret::new(String::from_utf16_lossy(slice));
        // Wipe the native buffer before releasing it: it can hold the plaintext key.
        std::ptr::write_bytes(xml.0, 0, len);
        WlanFreeMemory(xml.0 as *const core::ffi::c_void);
        owned
    };
    Ok(text)
}

/// Why reading a profile failed, kept separate from [`HostWifiError`] so the caller decides whether
/// a given status is a normal outcome or a genuine failure.
enum ProfileReadError {
    Status(u32),
}

/// List the profile names saved on one interface (§7.2).
fn profile_names(handle: &WlanHandle, interface: &GUID) -> Result<Vec<String>, HostWifiError> {
    let mut list: *mut WLAN_PROFILE_INFO_LIST = std::ptr::null_mut();
    let status = unsafe { WlanGetProfileList(handle.0, interface, None, &mut list) };
    win32_result(status, Operation::ResolveProfile)?;
    let _owned = WlanMemory(list);
    if list.is_null() {
        return Ok(Vec::new());
    }

    // SAFETY: on success the API returns a list whose ProfileInfo array has dwNumberOfItems entries;
    // the declared `[_; 1]` is the usual Win32 variable-length-array idiom.
    let names = unsafe {
        let count = (*list).dwNumberOfItems as usize;
        let items = (*list).ProfileInfo.as_ptr();
        (0..count)
            .map(|i| {
                let raw = &(*items.add(i)).strProfileName;
                let len = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
                String::from_utf16_lossy(&raw[..len])
            })
            .collect()
    };
    Ok(names)
}

/// Find the single saved profile whose SSID equals the connected one (§7.2).
///
/// Every profile is read *without* the plaintext flag, so resolution never collects anybody's keys.
/// Zero matches and more than one match are both refusals rather than a guess.
fn resolve_profile(
    handle: &WlanHandle,
    interface: &GUID,
    ssid: &str,
) -> Result<String, HostWifiError> {
    let mut matches = Vec::new();
    for name in profile_names(handle, interface)? {
        let Ok(xml) = get_profile_xml(handle, interface, &name, false) else {
            // A profile that cannot be read cannot be the match we can prove.
            continue;
        };
        let Some(profile) = wlan_profile::parse(&xml) else {
            continue;
        };
        if profile.ssid.matches(ssid) {
            matches.push(name);
        }
    }

    match matches.len() {
        0 => Err(HostWifiError::MalformedProfile),
        1 => Ok(matches.remove(0)),
        _ => Err(HostWifiError::AmbiguousProfile),
    }
}

pub(crate) fn read_saved_password(network: &NetworkRef) -> Result<PasswordOutcome, HostWifiError> {
    let NetworkRefInner::Windows {
        interface_guid,
        profile_name: ssid,
    } = &network.0
    else {
        return Err(HostWifiError::UnsupportedPlatform);
    };
    if interface_guid.is_empty() || ssid.is_empty() {
        return Ok(PasswordOutcome::NotStored);
    }

    let interface = parse_interface_guid(interface_guid)?;
    let handle = WlanHandle::open()?;

    // Discovery stores the SSID, not a profile name: the two are not interchangeable (§7.2).
    let profile_name = match resolve_profile(&handle, &interface, ssid) {
        Ok(name) => name,
        // No saved profile for this SSID is an ordinary situation, not a failure.
        Err(HostWifiError::MalformedProfile) => return Ok(PasswordOutcome::NotStored),
        Err(e) => return Err(e),
    };

    let xml = match get_profile_xml(&handle, &interface, &profile_name, true) {
        Ok(xml) => xml,
        Err(ProfileReadError::Status(status)) => {
            return match secret_outcome_for_status(status) {
                Some(outcome) => Ok(outcome),
                None => Err(HostWifiError::PlatformApi {
                    operation: Operation::ReadSecret,
                    code: status as i64,
                }),
            };
        }
    };

    let Some(profile) = wlan_profile::parse(&xml) else {
        return Err(HostWifiError::MalformedProfile);
    };

    Ok(outcome_from_profile(&profile))
}

/// Turn a parsed profile into the outcome the UI states (§7.3).
fn outcome_from_profile(profile: &wlan_profile::WlanProfile) -> PasswordOutcome {
    // The security type decides whether a portable passphrase can exist at all, before the key
    // material is even considered.
    match profile.security {
        SecurityKind::Open => return PasswordOutcome::NotRequired,
        SecurityKind::Enterprise | SecurityKind::UnsupportedSecurity => {
            return PasswordOutcome::UnsupportedSecurity;
        }
        SecurityKind::Personal | SecurityKind::Unknown => {}
    }

    match &profile.credential {
        wlan_profile::ProfileCredential::Plaintext(key) => {
            PasswordOutcome::Found(Secret::new(key.clone()))
        }
        // Still DPAPI-encrypted: the plaintext flag was not granted. Decrypting it is forbidden.
        wlan_profile::ProfileCredential::Encrypted => PasswordOutcome::PermissionDenied,
        wlan_profile::ProfileCredential::None => PasswordOutcome::NotStored,
        wlan_profile::ProfileCredential::Unusable => PasswordOutcome::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win32_statuses_map_to_the_right_error() {
        assert_eq!(win32_result(0, Operation::ReadSecret), Ok(()));
        assert_eq!(
            win32_result(5, Operation::ReadSecret),
            Err(HostWifiError::PermissionDenied {
                operation: Operation::ReadSecret
            })
        );
        assert_eq!(
            win32_result(1168, Operation::ReadSecret),
            Err(HostWifiError::PlatformApi {
                operation: Operation::ReadSecret,
                code: 1168
            })
        );
    }

    #[test]
    fn access_denied_on_the_secret_is_an_outcome_not_a_failure() {
        // A debug build running as asInvoker hits exactly this, and it must degrade to the manual
        // field rather than surfacing a platform error (§7.4).
        assert_eq!(
            secret_outcome_for_status(5),
            Some(PasswordOutcome::PermissionDenied)
        );
        assert_eq!(
            secret_outcome_for_status(1168),
            Some(PasswordOutcome::NotStored)
        );
        // Anything else stays a genuine failure.
        assert_eq!(secret_outcome_for_status(87), None);
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
    fn an_empty_ref_reports_nothing_stored_without_touching_the_api() {
        let empty = NetworkRef(NetworkRefInner::Windows {
            interface_guid: String::new(),
            profile_name: String::new(),
        });
        assert_eq!(read_saved_password(&empty), Ok(PasswordOutcome::NotStored));
    }

    #[test]
    fn profile_outcomes_follow_the_security_type_then_the_key() {
        use wlan_profile::{ProfileCredential, ProfileSsid, WlanProfile};

        let profile = |security, credential| WlanProfile {
            ssid: ProfileSsid::Name("HomeNet".to_owned()),
            security,
            credential,
        };

        // Open and enterprise are decided before the key material is looked at.
        assert_eq!(
            outcome_from_profile(&profile(SecurityKind::Open, ProfileCredential::None)),
            PasswordOutcome::NotRequired
        );
        assert_eq!(
            outcome_from_profile(&profile(SecurityKind::Enterprise, ProfileCredential::None)),
            PasswordOutcome::UnsupportedSecurity
        );
        assert_eq!(
            outcome_from_profile(&profile(
                SecurityKind::UnsupportedSecurity,
                ProfileCredential::Plaintext("wep-key-here".to_owned())
            )),
            PasswordOutcome::UnsupportedSecurity,
            "a WEP key must not be handed over even when it is readable"
        );

        // Personal networks are decided by the credential state.
        assert_eq!(
            outcome_from_profile(&profile(
                SecurityKind::Personal,
                ProfileCredential::Plaintext("hunter2-pass".to_owned())
            )),
            PasswordOutcome::Found(Secret::new("hunter2-pass"))
        );
        assert_eq!(
            outcome_from_profile(&profile(
                SecurityKind::Personal,
                ProfileCredential::Encrypted
            )),
            PasswordOutcome::PermissionDenied
        );
        assert_eq!(
            outcome_from_profile(&profile(SecurityKind::Personal, ProfileCredential::None)),
            PasswordOutcome::NotStored
        );
        assert_eq!(
            outcome_from_profile(&profile(
                SecurityKind::Personal,
                ProfileCredential::Unusable
            )),
            PasswordOutcome::Unavailable
        );
    }

    #[test]
    fn the_interface_guid_round_trips_through_the_network_ref() {
        // Discovery stores the GUID with Debug; retrieval must be able to parse that back.
        let guid = GUID::from_u128(0x1234_5678_9abc_def0_1234_5678_9abc_def0);
        let stored = format!("{guid:?}");
        assert_eq!(parse_interface_guid(&stored), Ok(guid));
        assert_eq!(
            parse_interface_guid("not-a-guid"),
            Err(HostWifiError::MalformedProfile)
        );
    }
}
