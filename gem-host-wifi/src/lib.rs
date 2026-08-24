
mod error;
mod model;
mod pal;

pub use error::{HostWifiError, Operation};
pub use model::{
    CountryCode, CountryHint, CountrySource, DetectedSsid, DetectedWifi, NetworkRef,
    PasswordOutcome, SecurityKind, is_usable_wifi_credential,
};

pub fn detect_current_wifi() -> Result<DetectedWifi, HostWifiError> {
    pal::detect_current_wifi()
}

pub fn prime_location_authorization() {
    pal::prime_location_authorization();
}

pub fn read_saved_password(network: &NetworkRef) -> Result<PasswordOutcome, HostWifiError> {
    pal::read_saved_password(network)
}

pub(crate) fn country_from_locale() -> Option<CountryHint> {
    fn region_from_locale(value: &str) -> Option<CountryCode> {
        let base = value.split(['.', '@']).next().unwrap_or(value);
        let region = base.split(['_', '-']).nth(1)?;
        CountryCode::parse(region)
    }

    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(key) {
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
