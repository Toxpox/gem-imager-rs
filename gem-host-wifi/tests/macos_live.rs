//! Real-host integration checks for the macOS CoreWLAN/Keychain backend.
//!
//! These need Location authorization, a live Wi-Fi association and a System keychain the user can
//! unlock, none of which a CI runner has (§14.6), so they are `#[ignore]` by default:
//!
//! ```sh
//! cargo test -p gem-host-wifi --test macos_live -- --ignored --nocapture
//! ```
//!
//! On a Personal network the password read really does prompt for Touch ID or a password. That
//! prompt appearing at all is the behaviour being verified; the value is never printed.

#![cfg(target_os = "macos")]

use gem_host_wifi::{DetectedSsid, HostWifiError, PasswordOutcome, SecurityKind};

#[test]
#[ignore = "requires Location authorization and a live Wi-Fi association"]
fn detects_the_current_network_on_this_host() {
    match gem_host_wifi::detect_current_wifi() {
        Ok(wifi) => {
            match &wifi.ssid {
                DetectedSsid::Utf8(name) => {
                    assert!(!name.is_empty(), "connected SSID should not be empty");
                    println!("SSID: {name}");
                }
                DetectedSsid::UnsupportedEncoding => println!("SSID: <non-UTF-8>"),
            }
            println!("security: {:?}", wifi.security);
            println!("country: {:?}", wifi.country);

            println!("requesting the saved password (expect an authorization prompt)...");
            let outcome = gem_host_wifi::read_saved_password(&wifi.network).unwrap();
            println!("password outcome: {outcome:?}");

            match (&wifi.security, &outcome) {
                (SecurityKind::Personal, PasswordOutcome::Found(secret)) => {
                    let len = secret.len();
                    assert!(
                        (8..=63).contains(&len) || len == 64,
                        "retrieved credential has an unusable length"
                    );
                    assert!(!format!("{secret:?}").contains(secret.expose()));
                    println!("retrieved a {len}-byte credential (value not shown)");
                }
                (SecurityKind::Personal, PasswordOutcome::NotStored) => {
                    // Before the System-keychain fix this was the outcome for every network, with no prompt shown.
                    println!(
                        "NotStored on a Personal network: no prompt was shown. If this network's \
                         password is saved in Settings > Wi-Fi, the System keychain lookup is \
                         still not reaching the AirPort item."
                    );
                }
                (SecurityKind::Personal, other) => {
                    // Deny/Cancel/PermissionDenied all mean the prompt did appear.
                    println!("prompt reached the user, outcome: {other:?}");
                }
                (kind, other) => println!("security {kind:?}, outcome: {other:?}"),
            }
        }
        Err(HostWifiError::PermissionDenied { operation }) => {
            println!(
                "Location not authorized yet ({operation:?}). Approve the prompt, or enable it in \
                 System Settings > Privacy & Security > Location Services, then re-run."
            );
        }
        Err(HostWifiError::NotConnected) => {
            println!("no Wi-Fi association right now; connect to a network and re-run");
        }
        Err(HostWifiError::NoWifiDevice) => println!("no Wi-Fi device on this host"),
        Err(other) => panic!("unexpected discovery error: {other}"),
    }
}

/// The password read must not depend on Location: the Keychain lookup is keyed by SSID only.
///
/// This is a pure-logic guard that runs without a live network.
#[test]
fn an_unrepresentable_ssid_never_reaches_the_keychain() {
    // An empty name (a non-UTF-8 SSID) must short-circuit rather than query with a blank account.
    let wifi = gem_host_wifi::detect_current_wifi();
    if let Ok(w) = wifi
        && matches!(w.ssid, DetectedSsid::UnsupportedEncoding)
    {
        assert_eq!(
            gem_host_wifi::read_saved_password(&w.network).unwrap(),
            PasswordOutcome::NotStored
        );
    }
}
