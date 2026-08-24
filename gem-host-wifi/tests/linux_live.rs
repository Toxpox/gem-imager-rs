#![cfg(target_os = "linux")]

use gem_host_wifi::{DetectedSsid, HostWifiError, PasswordOutcome, SecurityKind};

#[test]
#[ignore = "requires a live NetworkManager and Wi-Fi association"]
fn detects_the_current_network_on_this_host() {
    match gem_host_wifi::detect_current_wifi() {
        Ok(wifi) => {
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

            assert!(matches!(
                wifi.security,
                SecurityKind::Open
                    | SecurityKind::Personal
                    | SecurityKind::Enterprise
                    | SecurityKind::UnsupportedSecurity
                    | SecurityKind::Unknown
            ));

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
                    let len = secret.len();
                    assert!(
                        (8..=63).contains(&len) || len == 64,
                        "retrieved credential has an unusable length"
                    );
                    assert!(!format!("{secret:?}").contains(secret.expose()));
                    println!("retrieved a {len}-byte credential (value not shown)");
                }
                (SecurityKind::Personal, other) => {
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
