//! Real-host integration checks for the Linux NetworkManager backend.
//!
//! These need a live system D-Bus, NetworkManager and (for the connected case) an actual Wi-Fi
//! association, none of which a cloud CI runner has (research plan §14.6). They are therefore
//! `#[ignore]` by default and meant to be run by hand on a real machine:
//!
//! ```sh
//! cargo test -p gem-host-wifi --test linux_live -- --ignored --nocapture
//! ```
//!
//! They assert only structural facts and never print a password (there is none to print in Faz 1),
//! so they are safe to run and log.

#![cfg(target_os = "linux")]

use gem_host_wifi::{DetectedSsid, HostWifiError, PasswordOutcome, SecurityKind};

#[test]
#[ignore = "requires a live NetworkManager and Wi-Fi association"]
fn detects_the_current_network_on_this_host() {
    match gem_host_wifi::detect_current_wifi() {
        Ok(wifi) => {
            // If we are connected, the SSID must be a real value, not an empty string.
            match &wifi.ssid {
                DetectedSsid::Utf8(name) => {
                    assert!(!name.is_empty(), "connected SSID should not be empty");
                    println!("SSID: {name}");
                }
                DetectedSsid::UnsupportedEncoding => {
                    println!("SSID: <non-UTF-8>");
                }
            }
            println!("security: {:?}", wifi.security);
            println!("country: {:?}", wifi.country);

            // The security type should have been classified to something concrete on a real
            // connection; Unknown is allowed but noted.
            assert!(matches!(
                wifi.security,
                SecurityKind::Open
                    | SecurityKind::Personal
                    | SecurityKind::Enterprise
                    | SecurityKind::Unknown
            ));

            // Faz 1: password retrieval is intentionally not implemented yet.
            let outcome = gem_host_wifi::read_saved_password(&wifi.network).unwrap();
            assert_eq!(outcome, PasswordOutcome::Unavailable);
        }
        Err(HostWifiError::NotConnected) => {
            println!("no Wi-Fi association right now; connect to a network and re-run");
        }
        Err(HostWifiError::NoWifiDevice) => {
            println!("no Wi-Fi device on this host");
        }
        Err(other) => panic!("unexpected discovery error: {other}"),
    }
}
