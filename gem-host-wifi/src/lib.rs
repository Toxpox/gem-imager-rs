//! Read the host computer's current Wi-Fi network through native OS APIs only.
//!
//! The two questions are kept separate on purpose:
//!
//! 1. [`detect_current_wifi`] — what network is this computer on? Cheap, prompts nothing.
//! 2. [`read_saved_password`] — what is its saved passphrase? This is the step that may hit an OS
//!    permission or credential prompt, so the caller triggers it as a separate user action (§10.2).
//!
//! That split lets the UI fill in the SSID and country immediately while leaving the password to a
//! deliberate click, which matters most on macOS where reading it shows a Keychain prompt.
//!
//! No root file scraping, no credential-store database reading, no shelling out to `security`,
//! `netsh` or `nmcli --show-secrets`, no bypassing an OS permission. If the OS does not hand the
//! value over through a supported API, the answer is "ask the user" (§2.2).
//!
//! | Platform | SSID + country | Saved password |
//! |---|---|---|
//! | Linux + NetworkManager | yes | yes, via `GetSecrets` |
//! | Windows | yes | yes, via `WlanGetProfile` (needs the plaintext-key right) |
//! | macOS | yes | best effort, via a Keychain query that can prompt |
//! | Linux + standalone iwd / wpa_supplicant | SSID only | manual |
//!
//! Where a password cannot be produced the answer is a specific [`PasswordOutcome`], so the UI can
//! say what happened instead of showing a generic failure.

mod error;
mod model;
mod pal;

pub use error::{HostWifiError, Operation};
pub use model::{
    CountryCode, CountryHint, CountrySource, DetectedSsid, DetectedWifi, NetworkRef,
    PasswordOutcome, SecurityKind, is_usable_wifi_credential,
};

/// Detect the network the host is connected to right now.
///
/// [`HostWifiError::NotConnected`] means there is a Wi-Fi device but no active connection, and
/// [`HostWifiError::NoWifiDevice`] means there is no Wi-Fi at all. Both are ordinary situations the
/// UI turns into "type your network details".
pub fn detect_current_wifi() -> Result<DetectedWifi, HostWifiError> {
    pal::detect_current_wifi()
}

/// Ask the OS, once, for the permission host Wi-Fi discovery needs, so the answer is ready by the
/// time the user opens the Wi-Fi form.
///
/// Only macOS needs this, and there it does real work: Location authorization gates the SSID, the
/// grant is asynchronous, and CoreLocation delivers it to the **main thread's run loop**. Call this
/// from `main` on the main thread, before the UI event loop starts. Everywhere else it is a no-op.
///
/// Calling it is optional — [`detect_current_wifi`] primes the request itself if it has to — but
/// then the first detection races the user reading the permission dialog and comes back empty.
pub fn prime_location_authorization() {
    pal::prime_location_authorization();
}

/// Read the saved password for a previously [`detect_current_wifi`]-detected network.
///
/// Never call this from the toggle or on startup: it is the step that can prompt. The returned
/// [`PasswordOutcome`] distinguishes "open, no password needed" from "not stored" from "denied".
pub fn read_saved_password(network: &NetworkRef) -> Result<PasswordOutcome, HostWifiError> {
    pal::read_saved_password(network)
}

/// A last-resort country guess from the process locale, tagged [`CountrySource::Locale`].
///
/// Shared by every backend as the bottom of the country priority list (§9). Reads the environment
/// the way POSIX locale resolution does and returns `None` rather than a wrong guess.
pub(crate) fn country_from_locale() -> Option<CountryHint> {
    fn region_from_locale(value: &str) -> Option<CountryCode> {
        // `tr_TR.UTF-8` / `en_US@euro` -> `tr_TR` -> `TR`.
        let base = value.split(['.', '@']).next().unwrap_or(value);
        let region = base.split(['_', '-']).nth(1)?;
        CountryCode::parse(region)
    }

    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(key) {
            // "C" / "POSIX" carry no region; skip to the next source rather than failing.
            if let Some(code) = region_from_locale(&value) {
                return Some(CountryHint {
                    code,
                    source: CountrySource::Locale,
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_region_parsing_handles_common_shapes() {
        // All three locale vars are cleared first so the host cannot leak a region into the no-region cases.
        let saved: Vec<(&str, Option<String>)> = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .map(|&k| (k, std::env::var(k).ok()))
            .collect();
        // SAFETY: single-threaded test; every variable touched here is restored at the end.
        unsafe {
            for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
                std::env::remove_var(key);
            }
        }

        let cases = [
            ("tr_TR.UTF-8", Some("TR")),
            ("en_US.UTF-8", Some("US")),
            ("en_GB", Some("GB")),
            ("de_DE@euro", Some("DE")),
            ("C", None),
            ("POSIX", None),
            ("", None),
        ];
        for (value, expected) in cases {
            // SAFETY: single-threaded test; we set and restore one variable.
            unsafe {
                std::env::set_var("LC_ALL", value);
            }
            let got = country_from_locale();
            match expected {
                Some(code) => {
                    let hint = got.expect("expected a country hint");
                    assert_eq!(hint.code.as_str(), code, "for locale {value:?}");
                    assert_eq!(hint.source, CountrySource::Locale);
                }
                None => assert!(got.is_none(), "for locale {value:?} got {got:?}"),
            }
        }

        // Restore the caller's environment.
        unsafe {
            for (key, value) in saved {
                match value {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}
