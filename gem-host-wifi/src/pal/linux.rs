//! Linux backend: talk to NetworkManager over the system D-Bus.
//!
//! The whole discovery path is these NetworkManager objects (research plan §8.1, §8.2):
//!
//! ```text
//! org.freedesktop.NetworkManager
//!   -> a Wi-Fi Device (DeviceType == 2)
//!      -> Device.Wireless.ActiveAccessPoint -> AccessPoint.Ssid   (the SSID bytes)
//!      -> Device.ActiveConnection -> Connection.Active.Connection (the saved profile path)
//!         -> Settings.Connection.GetSettings -> 802-11-wireless-security.key-mgmt (security type)
//! ```
//!
//! `GetSettings` is used for the security *type* only; it never returns the passphrase. Reading the
//! actual secret needs `GetSecrets`, which is Faz 2. The country comes from `iw reg get` when it is
//! available (the regulatory domain, the authoritative source), otherwise from the locale.
//!
//! Everything here uses the generic untyped `blocking::Proxy` rather than a generated typed proxy:
//! the calls are few, and staying untyped keeps the backend working across NetworkManager versions
//! without pinning to a specific interface XML.

use std::collections::HashMap;

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

/// `NM_DEVICE_TYPE_WIFI`; DeviceType is a plain `u32` on the Device interface.
const DEVICE_TYPE_WIFI: u32 = 2;

fn platform_err(operation: Operation) -> impl Fn(zbus::Error) -> HostWifiError {
    move |e| match e {
        // NetworkManager raises this fdo error name when polkit/DACL blocks a call.
        zbus::Error::MethodError(ref name, _, _)
            if name.as_str() == "org.freedesktop.NetworkManager.PermissionDenied"
                || name.as_str() == "org.freedesktop.DBus.Error.AccessDenied" =>
        {
            HostWifiError::PermissionDenied { operation }
        }
        _ => HostWifiError::PlatformApi {
            operation,
            // D-Bus errors do not carry a numeric OS code; 0 keeps the error data-free (§12.3)
            // while the variant and operation already say what failed.
            code: 0,
        },
    }
}

/// Read one property off an object as an `OwnedValue`, going through `org.freedesktop.DBus.Properties`.
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

/// Find the first Wi-Fi device that has an active access point. Returns its device path.
fn active_wifi_device(conn: &Connection) -> Result<OwnedObjectPath, HostWifiError> {
    let nm = Proxy::new(conn, NM_SERVICE, NM_PATH, NM_IFACE)
        .map_err(platform_err(Operation::ListDevices))?;
    let devices: Vec<OwnedObjectPath> = nm
        .call("GetDevices", &())
        .map_err(platform_err(Operation::ListDevices))?;

    let mut saw_wifi = false;
    for dev in devices {
        let dev_str = dev.as_str();
        let dtype = get_property(conn, dev_str, DEVICE_IFACE, "DeviceType", Operation::ListDevices)
            .ok()
            .and_then(|v| u32::try_from(v).ok());
        if dtype != Some(DEVICE_TYPE_WIFI) {
            continue;
        }
        saw_wifi = true;

        let active_ap = get_property(conn, dev_str, WIRELESS_IFACE, "ActiveAccessPoint", Operation::ReadSsid)
            .ok()
            .and_then(|v| as_object_path(&v));
        // A path of "/" is NetworkManager's null object: a Wi-Fi radio that is not associated.
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

/// Read the SSID bytes off the device's active access point.
fn read_ssid(conn: &Connection, device: &OwnedObjectPath) -> Result<DetectedSsid, HostWifiError> {
    let ap = get_property(conn, device.as_str(), WIRELESS_IFACE, "ActiveAccessPoint", Operation::ReadSsid)?;
    let ap = as_object_path(&ap).ok_or(HostWifiError::NotConnected)?;
    if ap.as_str() == "/" {
        return Err(HostWifiError::NotConnected);
    }

    let ssid_value = get_property(conn, ap.as_str(), AP_IFACE, "Ssid", Operation::ReadSsid)?;
    let bytes: Vec<u8> = Vec::<u8>::try_from(ssid_value).map_err(|_| HostWifiError::InvalidSsidEncoding)?;
    Ok(DetectedSsid::from_bytes(&bytes))
}

/// Resolve the device's active connection to its saved `Settings.Connection` object path.
fn active_connection_path(
    conn: &Connection,
    device: &OwnedObjectPath,
) -> Result<Option<OwnedObjectPath>, HostWifiError> {
    let active = get_property(conn, device.as_str(), DEVICE_IFACE, "ActiveConnection", Operation::ReadSecret)?;
    let active = match as_object_path(&active) {
        Some(p) if p.as_str() != "/" => p,
        _ => return Ok(None),
    };

    let settings = get_property(conn, active.as_str(), ACTIVE_CONN_IFACE, "Connection", Operation::ReadSecret)?;
    Ok(as_object_path(&settings).filter(|p| p.as_str() != "/"))
}

/// Classify the security type from the connection's non-secret settings (§8.4).
///
/// `GetSettings` returns everything *except* secrets, which is exactly enough to read
/// `802-11-wireless-security.key-mgmt` and decide whether a single portable passphrase can exist.
fn read_security(
    conn: &Connection,
    settings_path: &OwnedObjectPath,
) -> Result<SecurityKind, HostWifiError> {
    let proxy = Proxy::new(conn, NM_SERVICE, settings_path.as_str(), SETTINGS_CONN_IFACE)
        .map_err(platform_err(Operation::ReadSecret))?;
    let settings: HashMap<String, HashMap<String, OwnedValue>> = proxy
        .call("GetSettings", &())
        .map_err(platform_err(Operation::ReadSecret))?;

    let Some(security) = settings.get("802-11-wireless-security") else {
        // No security block at all means an open network.
        return Ok(SecurityKind::Open);
    };

    let key_mgmt = security
        .get("key-mgmt")
        .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
        .unwrap_or_default();

    Ok(classify_key_mgmt(&key_mgmt))
}

/// Map a NetworkManager `key-mgmt` value to a [`SecurityKind`].
///
/// Kept as a free function so the whole classification table is unit-testable without a live bus.
fn classify_key_mgmt(key_mgmt: &str) -> SecurityKind {
    match key_mgmt {
        // A single portable passphrase: WPA/WPA2-PSK and WPA3-SAE.
        "wpa-psk" | "sae" => SecurityKind::Personal,
        // No portable per-user secret.
        "wpa-eap" | "wpa-eap-suite-b-192" | "ieee8021x" => SecurityKind::Enterprise,
        // Opportunistic Wireless Encryption is "open" with no passphrase to carry.
        "owe" => SecurityKind::Open,
        "none" | "" => SecurityKind::Open,
        _ => SecurityKind::Unknown,
    }
}

/// The regulatory country from `iw reg get`, the authoritative source (§9). `iw` is a standard,
/// unprivileged read; parsing its `country XX:` line avoids a raw nl80211 netlink dependency.
fn country_from_iw() -> Option<CountryHint> {
    let output = std::process::Command::new("iw").args(["reg", "get"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let line = line.trim();
        // Lines look like: "country TR: DFS-ETSI" or, when unset, "country 00: DFS-UNSET".
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

/// Regulatory domain first, then locale, matching the Linux priority in the plan (§9).
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
        // Connected to an AP but with no saved profile object (rare): keep the SSID, and mark the
        // network with the null path so a later password request cleanly reports NotStored.
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

pub(crate) fn read_saved_password(
    _network: &NetworkRef,
) -> Result<PasswordOutcome, HostWifiError> {
    // Faz 2 wires up Settings.Connection.GetSecrets("802-11-wireless-security"). Until then, be
    // honest rather than guess: the UI keeps the manual password field.
    Ok(PasswordOutcome::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_mgmt_classification_covers_the_supported_matrix() {
        assert_eq!(classify_key_mgmt("wpa-psk"), SecurityKind::Personal);
        assert_eq!(classify_key_mgmt("sae"), SecurityKind::Personal);
        assert_eq!(classify_key_mgmt("wpa-eap"), SecurityKind::Enterprise);
        assert_eq!(classify_key_mgmt("ieee8021x"), SecurityKind::Enterprise);
        assert_eq!(classify_key_mgmt("owe"), SecurityKind::Open);
        assert_eq!(classify_key_mgmt("none"), SecurityKind::Open);
        assert_eq!(classify_key_mgmt(""), SecurityKind::Open);
        assert_eq!(classify_key_mgmt("some-future-scheme"), SecurityKind::Unknown);
    }
}
