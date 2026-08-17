//! Read the host computer's current Wi-Fi network through native OS APIs only.
//!
//! This crate answers two questions, and keeps them separate on purpose:
//!
//! 1. [`detect_current_wifi`] — what network is this computer on right now (SSID, security type,
//!    a best-effort country)? This is cheap and prompts nothing.
//! 2. [`read_saved_password`] — for a network we already detected, what is its saved passphrase?
//!    This is the step that may hit an OS permission or credential prompt, so the caller triggers
//!    it as a separate, explicit user action (research plan §10.2).
//!
//! The split is what lets the UI fill in the SSID and country immediately while leaving the
//! password to a deliberate "import password" click — which matters most on macOS, where reading
//! the password shows a Keychain prompt that should line up with the user asking for it.
//!
//! # What this crate will not do
//!
//! No root file scraping, no credential-store database reading, no shelling out to `security`,
//! `netsh` or `nmcli --show-secrets`, no bypassing an OS permission. If the OS does not hand the
//! value over through a supported API, the answer is "ask the user", not a workaround (§2.2).
//!
//! # Platform support
//!
//! | Platform | SSID + country | Saved password |
//! |---|---|---|
//! | Linux + NetworkManager | yes | yes (Faz 2) |
//! | Windows | yes | yes (Faz 2) |
//! | macOS | yes | best effort (Faz 2) |
//! | Linux + standalone iwd / wpa_supplicant | SSID only | manual |
//!
//! This module is Faz 1: discovery. Password retrieval lands in Faz 2; until then
//! [`read_saved_password`] reports [`PasswordOutcome::Unavailable`] rather than pretending.

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
/// Returns [`HostWifiError::NotConnected`] when there is a Wi-Fi device but no active connection,
/// and [`HostWifiError::NoWifiDevice`] when there is no Wi-Fi at all. Both are ordinary situations
/// the UI turns into "type your network details".
pub fn detect_current_wifi() -> Result<DetectedWifi, HostWifiError> {
    pal::detect_current_wifi()
}

/// Read the saved password for a previously [`detect_current_wifi`]-detected network.
///
/// Never call this from the toggle or on startup: it is the step that can prompt. The returned
/// [`PasswordOutcome`] distinguishes "open, no password needed" from "not stored" from "denied", so
/// the UI can say the right thing instead of showing a generic failure.
pub fn read_saved_password(network: &NetworkRef) -> Result<PasswordOutcome, HostWifiError> {
    pal::read_saved_password(network)
}

/// A last-resort country guess from the process locale, tagged [`CountrySource::Locale`].
///
/// Shared by every backend as the bottom of the country priority list (§9). It reads the
/// environment the way POSIX locale resolution does (`LC_ALL`, then `LC_MESSAGES`, then `LANG`) and
/// pulls a region out of values like `tr_TR.UTF-8`. Returns `None` rather than a wrong guess when
/// no region can be found, so the field is simply left for the user.
pub(crate) fn country_from_locale() -> Option<CountryHint> {
    fn region_from_locale(value: &str) -> Option<CountryCode> {
        // Strip an encoding/modifier suffix: `tr_TR.UTF-8` / `en_US@euro` -> `tr_TR`.
        let base = value
            .split(['.', '@'])
            .next()
            .unwrap_or(value);
        // The region is the part after `_` (or `-`): `tr_TR` -> `TR`.
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
        // Exercised through the real resolution path. All three locale vars are cleared first so the
        // host's own environment cannot leak a region into the "no region" cases.
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
