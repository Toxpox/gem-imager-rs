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
                    | SecurityKind::UnsupportedSecurity
                    | SecurityKind::Unknown
            ));

            // Password retrieval against the live secret agent. The *outcome* is printed, never the
            // password: on a Personal network this really does read the host's passphrase, so the
            // assertions below only ever look at its length and shape.
            let outcome = gem_host_wifi::read_saved_password(&wifi.network).unwrap();
            println!("password outcome: {outcome:?}");

            match (&wifi.security, &outcome) {
                (SecurityKind::Open, got) => assert_eq!(
                    *got,
                    PasswordOutcome::NotRequired,
                    "an open network needs no password"
                ),
                (SecurityKind::Enterprise | SecurityKind::UnsupportedSecurity, got) => {
                    assert_eq!(
                        *got,
                        PasswordOutcome::UnsupportedSecurity,
                        "no single portable passphrase exists for this security type"
                    );
                }
                (SecurityKind::Personal, PasswordOutcome::Found(secret)) => {
                    // Only the shape is asserted, so a failure message cannot carry the value.
                    let len = secret.len();
                    assert!(
                        (8..=63).contains(&len) || len == 64,
                        "retrieved credential has an unusable length"
                    );
                    assert!(!format!("{secret:?}").contains(secret.expose()));
                    println!("retrieved a {len}-byte credential (value not shown)");
                }
                (SecurityKind::Personal, other) => {
                    // Legitimate on a confined install, a not-saved profile or a headless session.
                    println!("no password available for this Personal network: {other:?}");
                }
                (SecurityKind::Unknown, other) => {
                    println!("unclassified security, outcome: {other:?}");
                }
            }
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
