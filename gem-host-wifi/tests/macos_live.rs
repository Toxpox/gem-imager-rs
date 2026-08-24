
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
                    println!(
                        "NotStored on a Personal network: no prompt was shown. If this network's \
                         password is saved in Settings > Wi-Fi, the System keychain lookup is \
                         still not reaching the AirPort item."
                    );
                }
                (SecurityKind::Personal, other) => {
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

#[test]
fn an_unrepresentable_ssid_never_reaches_the_keychain() {
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
