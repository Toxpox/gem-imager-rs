use std::collections::HashMap;

use gem_helper::secret::Secret;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use crate::error::Operation;
use crate::model::{
    CountryCode, CountryHint, CountrySource, DetectedSsid, DetectedWifi, NetworkRef,
    NetworkRefInner, SecurityKind,
};
use crate::{HostWifiError, PasswordOutcome};

const NM_SERVICE: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const NM_IFACE: &str = "org.freedesktop.NetworkManager";
const DEVICE_IFACE: &str = "org.freedesktop.NetworkManager.Device";
const WIRELESS_IFACE: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const AP_IFACE: &str = "org.freedesktop.NetworkManager.AccessPoint";
const ACTIVE_CONN_IFACE: &str = "org.freedesktop.NetworkManager.Connection.Active";
const SETTINGS_CONN_IFACE: &str = "org.freedesktop.NetworkManager.Settings.Connection";

const DEVICE_TYPE_WIFI: u32 = 2;

const SECURITY_SETTING: &str = "802-11-wireless-security";
const PSK_FIELD: &str = "psk";
const PSK_FLAGS_FIELD: &str = "psk-flags";

const SECRET_FLAG_NOT_SAVED: u32 = 0x02;
const SECRET_FLAG_NOT_REQUIRED: u32 = 0x04;

fn platform_err(operation: Operation) -> impl Fn(zbus::Error) -> HostWifiError {
    move |e| match e {
        zbus::Error::MethodError(ref name, _, _)
            if name.as_str() == "org.freedesktop.NetworkManager.PermissionDenied"
                || name.as_str() == "org.freedesktop.DBus.Error.AccessDenied" =>
        {
            HostWifiError::PermissionDenied { operation }
        }
        _ => HostWifiError::PlatformApi {
            operation,
            code: 0,
        },
    }
}

fn get_property(
    conn: &Connection,
    path: &str,
    interface: &str,
    property: &str,
    operation: Operation,
) -> Result<OwnedValue, HostWifiError> {
    let proxy = Proxy::new(conn, NM_SERVICE, path, "org.freedesktop.DBus.Properties")
        .map_err(platform_err(operation))?;
    proxy
        .call("Get", &(interface, property))
        .map_err(platform_err(operation))
}

fn as_object_path(value: &OwnedValue) -> Option<OwnedObjectPath> {
    OwnedObjectPath::try_from(value.try_clone().ok()?).ok()
}

fn active_wifi_device(conn: &Connection) -> Result<OwnedObjectPath, HostWifiError> {
    let nm = Proxy::new(conn, NM_SERVICE, NM_PATH, NM_IFACE)
        .map_err(platform_err(Operation::ListDevices))?;
    let devices: Vec<OwnedObjectPath> = nm
        .call("GetDevices", &())
        .map_err(platform_err(Operation::ListDevices))?;

    let mut saw_wifi = false;
    for dev in devices {
        let dev_str = dev.as_str();
        let dtype = get_property(
            conn,
            dev_str,
            DEVICE_IFACE,
            "DeviceType",
            Operation::ListDevices,
        )
        .ok()
        .and_then(|v| u32::try_from(v).ok());
        if dtype != Some(DEVICE_TYPE_WIFI) {
            continue;
        }
        saw_wifi = true;

        let active_ap = get_property(
            conn,
            dev_str,
            WIRELESS_IFACE,
            "ActiveAccessPoint",
            Operation::ReadSsid,
        )
        .ok()
        .and_then(|v| as_object_path(&v));
        if let Some(ap) = active_ap
            && ap.as_str() != "/"
        {
            return Ok(dev);
        }
    }

    Err(if saw_wifi {
        HostWifiError::NotConnected
    } else {
        HostWifiError::NoWifiDevice
    })
}

fn read_ssid(conn: &Connection, device: &OwnedObjectPath) -> Result<DetectedSsid, HostWifiError> {
    let ap = get_property(
        conn,
        device.as_str(),
        WIRELESS_IFACE,
        "ActiveAccessPoint",
        Operation::ReadSsid,
    )?;
    let ap = as_object_path(&ap).ok_or(HostWifiError::NotConnected)?;
    if ap.as_str() == "/" {
        return Err(HostWifiError::NotConnected);
    }

    let ssid_value = get_property(conn, ap.as_str(), AP_IFACE, "Ssid", Operation::ReadSsid)?;
    let bytes: Vec<u8> =
        Vec::<u8>::try_from(ssid_value).map_err(|_| HostWifiError::InvalidSsidEncoding)?;
    Ok(DetectedSsid::from_bytes(&bytes))
}

fn active_connection_path(
    conn: &Connection,
    device: &OwnedObjectPath,
) -> Result<Option<OwnedObjectPath>, HostWifiError> {
    let active = get_property(
        conn,
        device.as_str(),
        DEVICE_IFACE,
        "ActiveConnection",
        Operation::ReadSecret,
    )?;
    let active = match as_object_path(&active) {
        Some(p) if p.as_str() != "/" => p,
        _ => return Ok(None),
    };

    let settings = get_property(
        conn,
        active.as_str(),
        ACTIVE_CONN_IFACE,
        "Connection",
        Operation::ReadSecret,
    )?;
    Ok(as_object_path(&settings).filter(|p| p.as_str() != "/"))
}

fn read_security(
    conn: &Connection,
    settings_path: &OwnedObjectPath,
) -> Result<SecurityKind, HostWifiError> {
    let settings = get_settings(conn, settings_path)?;
    Ok(security_from_settings(&settings))
}

fn get_settings(
    conn: &Connection,
    settings_path: &OwnedObjectPath,
) -> Result<HashMap<String, HashMap<String, OwnedValue>>, HostWifiError> {
    let proxy = Proxy::new(
        conn,
        NM_SERVICE,
        settings_path.as_str(),
        SETTINGS_CONN_IFACE,
    )
    .map_err(platform_err(Operation::ReadSecret))?;
    proxy
        .call("GetSettings", &())
        .map_err(platform_err(Operation::ReadSecret))
}

fn security_from_settings(settings: &HashMap<String, HashMap<String, OwnedValue>>) -> SecurityKind {
    let Some(security) = settings.get(SECURITY_SETTING) else {
        return SecurityKind::Open;
    };

    let key_mgmt = security
        .get("key-mgmt")
        .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
        .unwrap_or_default();

    classify_key_mgmt(&key_mgmt, true)
}

fn classify_key_mgmt(key_mgmt: &str, has_security_block: bool) -> SecurityKind {
    match key_mgmt {
        "wpa-psk" | "sae" => SecurityKind::Personal,
        "wpa-eap" | "wpa-eap-suite-b-192" | "ieee8021x" => SecurityKind::Enterprise,
        "owe" => SecurityKind::Open,
        "none" | "" => {
            if has_security_block {
                SecurityKind::UnsupportedSecurity
            } else {
                SecurityKind::Open
            }
        }
        _ => SecurityKind::Unknown,
    }
}

fn country_from_iw() -> Option<CountryHint> {
    let output = std::process::Command::new("iw")
        .args(["reg", "get"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("country ") {
            let code = rest.split(':').next().unwrap_or("").trim();
            if let Some(parsed) = CountryCode::parse(code) {
                return Some(CountryHint {
                    code: parsed,
                    source: CountrySource::Regulatory,
                });
            }
        }
    }
    None
}

fn detect_country() -> Option<CountryHint> {
    country_from_iw().or_else(crate::country_from_locale)
}

pub(crate) fn detect_current_wifi() -> Result<DetectedWifi, HostWifiError> {
    let conn = Connection::system().map_err(platform_err(Operation::ListDevices))?;

    let device = active_wifi_device(&conn)?;
    let ssid = read_ssid(&conn, &device)?;

    let (network, security) = match active_connection_path(&conn, &device)? {
        Some(settings_path) => {
            let security = read_security(&conn, &settings_path).unwrap_or(SecurityKind::Unknown);
            let network = NetworkRef(NetworkRefInner::NetworkManager {
                connection_path: settings_path.as_str().to_owned(),
            });
            (network, security)
        }
        None => (
            NetworkRef(NetworkRefInner::NetworkManager {
                connection_path: "/".to_owned(),
            }),
            SecurityKind::Unknown,
        ),
    };

    Ok(DetectedWifi {
        network,
        ssid,
        security,
        country: detect_country(),
    })
}

fn outcome_from_psk_flags(flags: u32) -> Option<PasswordOutcome> {
    if flags & SECRET_FLAG_NOT_SAVED != 0 {
        return Some(PasswordOutcome::NotStored);
    }
    if flags & SECRET_FLAG_NOT_REQUIRED != 0 {
        return Some(PasswordOutcome::NotRequired);
    }
    None
}

fn classify_secrets_error(error: &zbus::Error) -> Result<PasswordOutcome, HostWifiError> {
    let zbus::Error::MethodError(name, _, _) = error else {
        return Err(HostWifiError::PlatformApi {
            operation: Operation::ReadSecret,
            code: 0,
        });
    };

    Ok(match name.as_str() {
        "org.freedesktop.NetworkManager.Settings.Connection.SettingNotFound" => {
            PasswordOutcome::NotRequired
        }
        "org.freedesktop.NetworkManager.AgentManager.NoSecrets" => PasswordOutcome::NotStored,
        "org.freedesktop.NetworkManager.AgentManager.UserCanceled" => {
            PasswordOutcome::UserCancelled
        }
        "org.freedesktop.NetworkManager.AgentManager.PermissionDenied"
        | "org.freedesktop.NetworkManager.PermissionDenied"
        | "org.freedesktop.DBus.Error.AccessDenied" => PasswordOutcome::PermissionDenied,
        _ => {
            return Err(HostWifiError::PlatformApi {
                operation: Operation::ReadSecret,
                code: 0,
            });
        }
    })
}

fn psk_from_secrets(secrets: &HashMap<String, HashMap<String, OwnedValue>>) -> PasswordOutcome {
    let Some(security) = secrets.get(SECURITY_SETTING) else {
        return PasswordOutcome::NotStored;
    };
    let Some(value) = security.get(PSK_FIELD) else {
        return PasswordOutcome::NotStored;
    };
    let Ok(psk) = value.try_clone().and_then(String::try_from) else {
        return PasswordOutcome::Unavailable;
    };

    if !crate::is_usable_wifi_credential(&psk) {
        return PasswordOutcome::Unavailable;
    }
    PasswordOutcome::Found(Secret::new(psk))
}

pub(crate) fn read_saved_password(network: &NetworkRef) -> Result<PasswordOutcome, HostWifiError> {
    let NetworkRefInner::NetworkManager { connection_path } = &network.0 else {
        return Err(HostWifiError::UnsupportedPlatform);
    };
    if connection_path == "/" {
        return Ok(PasswordOutcome::NotStored);
    }

    let conn = Connection::system().map_err(platform_err(Operation::ReadSecret))?;
    let settings_path = OwnedObjectPath::try_from(connection_path.as_str())
        .map_err(|_| HostWifiError::MalformedProfile)?;

    let settings = get_settings(&conn, &settings_path)?;
    let security = security_from_settings(&settings);
    match security {
        SecurityKind::Open => return Ok(PasswordOutcome::NotRequired),
        SecurityKind::Enterprise | SecurityKind::UnsupportedSecurity => {
            return Ok(PasswordOutcome::UnsupportedSecurity);
        }
        SecurityKind::Personal | SecurityKind::Unknown => {}
    }

    if let Some(flags) = settings
        .get(SECURITY_SETTING)
        .and_then(|s| s.get(PSK_FLAGS_FIELD))
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| u32::try_from(v).ok())
        && let Some(outcome) = outcome_from_psk_flags(flags)
    {
        return Ok(outcome);
    }

    let proxy = Proxy::new(
        &conn,
        NM_SERVICE,
        settings_path.as_str(),
        SETTINGS_CONN_IFACE,
    )
    .map_err(platform_err(Operation::ReadSecret))?;
    let secrets: HashMap<String, HashMap<String, OwnedValue>> =
        match proxy.call("GetSecrets", &(SECURITY_SETTING,)) {
            Ok(secrets) => secrets,
            Err(e) => return classify_secrets_error(&e),
        };

    Ok(psk_from_secrets(&secrets))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_map(
        entries: &[(&str, &[(&str, OwnedValue)])],
    ) -> HashMap<String, HashMap<String, OwnedValue>> {
        entries
            .iter()
            .map(|(group, fields)| {
                let inner = fields
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), v.try_clone().expect("clone value")))
                    .collect();
                ((*group).to_owned(), inner)
            })
            .collect()
    }

    fn str_value(s: &str) -> OwnedValue {
        zbus::zvariant::Value::from(s)
            .try_into()
            .expect("string value")
    }

    fn u32_value(n: u32) -> OwnedValue {
        zbus::zvariant::Value::from(n)
            .try_into()
            .expect("u32 value")
    }

    #[test]
    fn key_mgmt_classification_covers_the_supported_matrix() {
        assert_eq!(classify_key_mgmt("wpa-psk", true), SecurityKind::Personal);
        assert_eq!(classify_key_mgmt("sae", true), SecurityKind::Personal);
        assert_eq!(classify_key_mgmt("wpa-eap", true), SecurityKind::Enterprise);
        assert_eq!(
            classify_key_mgmt("ieee8021x", true),
            SecurityKind::Enterprise
        );
        assert_eq!(classify_key_mgmt("owe", true), SecurityKind::Open);
        assert_eq!(
            classify_key_mgmt("some-future-scheme", true),
            SecurityKind::Unknown
        );
    }

    #[test]
    fn key_mgmt_none_is_wep_with_a_security_block_and_open_without_one() {
        assert_eq!(
            classify_key_mgmt("none", true),
            SecurityKind::UnsupportedSecurity
        );
        assert_eq!(classify_key_mgmt("none", false), SecurityKind::Open);
        assert_eq!(classify_key_mgmt("", false), SecurityKind::Open);
    }

    #[test]
    fn security_is_open_when_the_settings_have_no_security_group() {
        let settings = settings_map(&[("802-11-wireless", &[("ssid", str_value("HomeNet"))])]);
        assert_eq!(security_from_settings(&settings), SecurityKind::Open);
    }

    #[test]
    fn security_reads_key_mgmt_out_of_the_security_group() {
        let settings = settings_map(&[(SECURITY_SETTING, &[("key-mgmt", str_value("wpa-psk"))])]);
        assert_eq!(security_from_settings(&settings), SecurityKind::Personal);

        let sae = settings_map(&[(SECURITY_SETTING, &[("key-mgmt", str_value("sae"))])]);
        assert_eq!(security_from_settings(&sae), SecurityKind::Personal);

        let eap = settings_map(&[(SECURITY_SETTING, &[("key-mgmt", str_value("wpa-eap"))])]);
        assert_eq!(security_from_settings(&eap), SecurityKind::Enterprise);

        let bare = settings_map(&[(SECURITY_SETTING, &[])]);
        assert_eq!(
            security_from_settings(&bare),
            SecurityKind::UnsupportedSecurity
        );
    }

    #[test]
    fn psk_flags_short_circuit_only_not_saved_and_not_required() {
        assert_eq!(outcome_from_psk_flags(0x00), None);
        assert_eq!(outcome_from_psk_flags(0x01), None);
        assert_eq!(
            outcome_from_psk_flags(0x02),
            Some(PasswordOutcome::NotStored)
        );
        assert_eq!(
            outcome_from_psk_flags(0x04),
            Some(PasswordOutcome::NotRequired)
        );
        assert_eq!(
            outcome_from_psk_flags(0x01 | 0x02),
            Some(PasswordOutcome::NotStored)
        );
        assert_eq!(
            outcome_from_psk_flags(0x02 | 0x04),
            Some(PasswordOutcome::NotStored)
        );
    }

    #[test]
    fn a_psk_in_the_reply_becomes_a_secret() {
        let secrets = settings_map(&[(SECURITY_SETTING, &[("psk", str_value("hunter2-pass"))])]);
        match psk_from_secrets(&secrets) {
            PasswordOutcome::Found(secret) => {
                assert_eq!(secret.expose(), "hunter2-pass");
                assert!(!format!("{secret:?}").contains("hunter2"));
            }
            other => panic!("expected Found, got {other:?}"),
        }

        let psk = "0DC0D6EB90555ED6419756B9A15EC3E3209B63DF707DD508D14581F8982721AF";
        let secrets = settings_map(&[(SECURITY_SETTING, &[("psk", str_value(psk))])]);
        assert_eq!(
            psk_from_secrets(&secrets),
            PasswordOutcome::Found(Secret::new(psk))
        );
    }

    #[test]
    fn a_reply_without_a_usable_psk_is_reported_not_guessed() {
        let empty = settings_map(&[]);
        assert_eq!(psk_from_secrets(&empty), PasswordOutcome::NotStored);

        let no_psk = settings_map(&[(SECURITY_SETTING, &[("key-mgmt", str_value("wpa-psk"))])]);
        assert_eq!(psk_from_secrets(&no_psk), PasswordOutcome::NotStored);

        for bad in ["short", &"z".repeat(64), &"a".repeat(70)] {
            let secrets = settings_map(&[(SECURITY_SETTING, &[("psk", str_value(bad))])]);
            assert_eq!(
                psk_from_secrets(&secrets),
                PasswordOutcome::Unavailable,
                "value of length {} should be rejected",
                bad.len()
            );
        }

        let wrong_type = settings_map(&[(SECURITY_SETTING, &[("psk", u32_value(1234))])]);
        assert_eq!(psk_from_secrets(&wrong_type), PasswordOutcome::Unavailable);
    }

    #[test]
    fn get_secrets_errors_map_to_outcomes_or_stay_errors() {
        fn method_error(name: &'static str) -> zbus::Error {
            zbus::Error::MethodError(
                zbus::names::OwnedErrorName::try_from(name).expect("valid error name"),
                None,
                zbus::message::Message::method_call("/", "Whatever")
                    .expect("builder")
                    .destination("org.example")
                    .expect("destination")
                    .interface("org.example")
                    .expect("interface")
                    .build(&())
                    .expect("message"),
            )
        }

        assert_eq!(
            classify_secrets_error(&method_error(
                "org.freedesktop.NetworkManager.Settings.Connection.SettingNotFound"
            )),
            Ok(PasswordOutcome::NotRequired)
        );
        assert_eq!(
            classify_secrets_error(&method_error(
                "org.freedesktop.NetworkManager.AgentManager.NoSecrets"
            )),
            Ok(PasswordOutcome::NotStored)
        );
        assert_eq!(
            classify_secrets_error(&method_error(
                "org.freedesktop.NetworkManager.AgentManager.UserCanceled"
            )),
            Ok(PasswordOutcome::UserCancelled)
        );
        assert_eq!(
            classify_secrets_error(&method_error(
                "org.freedesktop.NetworkManager.AgentManager.PermissionDenied"
            )),
            Ok(PasswordOutcome::PermissionDenied)
        );
        assert_eq!(
            classify_secrets_error(&method_error("org.freedesktop.DBus.Error.AccessDenied")),
            Ok(PasswordOutcome::PermissionDenied)
        );

        assert_eq!(
            classify_secrets_error(&method_error("org.freedesktop.DBus.Error.NoReply")),
            Err(HostWifiError::PlatformApi {
                operation: Operation::ReadSecret,
                code: 0,
            })
        );
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
    fn the_null_connection_path_means_nothing_is_stored() {
        let unsaved = NetworkRef(NetworkRefInner::NetworkManager {
            connection_path: "/".to_owned(),
        });
        assert_eq!(
            read_saved_password(&unsaved),
            Ok(PasswordOutcome::NotStored)
        );
    }
}
