
use gem_helper::secret::Secret;

#[derive(Clone, PartialEq, Eq)]
pub struct NetworkRef(pub(crate) NetworkRefInner);

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum NetworkRefInner {
    #[cfg(target_os = "linux")]
    NetworkManager { connection_path: String },
    #[cfg(target_os = "windows")]
    Windows {
        interface_guid: String,
        profile_name: String,
    },
    #[cfg(target_os = "macos")]
    MacOs { ssid: String },
    #[cfg_attr(
        any(target_os = "linux", target_os = "windows", target_os = "macos"),
        allow(dead_code)
    )]
    Opaque(String),
}

impl std::fmt::Debug for NetworkRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self.0 {
            #[cfg(target_os = "linux")]
            NetworkRefInner::NetworkManager { .. } => "NetworkManager",
            #[cfg(target_os = "windows")]
            NetworkRefInner::Windows { .. } => "Windows",
            #[cfg(target_os = "macos")]
            NetworkRefInner::MacOs { .. } => "MacOs",
            NetworkRefInner::Opaque(_) => "Opaque",
        };
        write!(f, "NetworkRef({kind}, <redacted>)")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedWifi {
    pub network: NetworkRef,
    pub ssid: DetectedSsid,
    pub security: SecurityKind,
    pub country: Option<CountryHint>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DetectedSsid {
    Utf8(String),
    UnsupportedEncoding,
}

impl DetectedSsid {
    pub fn as_utf8(&self) -> Option<&str> {
        match self {
            Self::Utf8(s) => Some(s),
            Self::UnsupportedEncoding => None,
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        match std::str::from_utf8(bytes) {
            Ok(s) => Self::Utf8(s.to_owned()),
            Err(_) => Self::UnsupportedEncoding,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecurityKind {
    Open,
    Personal,
    Enterprise,
    UnsupportedSecurity,
    Unknown,
}

impl SecurityKind {
    pub fn can_carry_passphrase(self) -> bool {
        matches!(self, Self::Personal)
    }
}

const WPA_PASSPHRASE_MIN_LEN: usize = 8;
const WPA_PASSPHRASE_MAX_LEN: usize = 63;
const WPA_PSK_HEX_LEN: usize = 64;

pub fn is_usable_wifi_credential(value: &str) -> bool {
    match value.len() {
        WPA_PSK_HEX_LEN => value.bytes().all(|b| b.is_ascii_hexdigit()),
        WPA_PASSPHRASE_MIN_LEN..=WPA_PASSPHRASE_MAX_LEN => true,
        _ => false,
    }
}

#[derive(Debug)]
pub enum PasswordOutcome {
    Found(Secret),
    NotRequired,
    NotStored,
    PermissionDenied,
    UserDenied,
    UserCancelled,
    UnsupportedSecurity,
    Unavailable,
}

impl PartialEq for PasswordOutcome {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryHint {
    pub code: CountryCode,
    pub source: CountrySource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CountrySource {
    Regulatory,
    AccessPoint,
    UserRegion,
    Locale,
}

impl CountrySource {
    pub fn is_authoritative(self) -> bool {
        matches!(self, Self::Regulatory)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CountryCode([u8; 2]);

impl CountryCode {
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        let bytes = raw.as_bytes();
        if bytes.len() != 2 || !bytes.iter().all(u8::is_ascii_alphabetic) {
            return None;
        }
        Some(Self([
            bytes[0].to_ascii_uppercase(),
            bytes[1].to_ascii_uppercase(),
        ]))
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or("")
    }
}

impl std::fmt::Debug for CountryCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CountryCode({})", self.as_str())
    }
}

impl std::fmt::Display for CountryCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssid_classifies_utf8_and_non_utf8_bytes() {
        assert_eq!(
            DetectedSsid::from_bytes("HomeNet".as_bytes()),
            DetectedSsid::Utf8("HomeNet".to_owned())
        );
        assert_eq!(
            DetectedSsid::from_bytes("Café-Ağ".as_bytes()),
            DetectedSsid::Utf8("Café-Ağ".to_owned())
        );
        assert_eq!(
            DetectedSsid::from_bytes(&[b'n', b'e', b't', 0xFF]),
            DetectedSsid::UnsupportedEncoding
        );
        assert_eq!(DetectedSsid::UnsupportedEncoding.as_utf8(), None);
    }

    #[test]
    fn country_code_normalises_and_rejects() {
        assert_eq!(CountryCode::parse("tr").unwrap().as_str(), "TR");
        assert_eq!(CountryCode::parse("  us ").unwrap().as_str(), "US");
        assert_eq!(CountryCode::parse("DE").unwrap().as_str(), "DE");
        assert!(CountryCode::parse("00").is_none());
        assert!(CountryCode::parse("C").is_none());
        assert!(CountryCode::parse("en_US").is_none());
        assert!(CountryCode::parse("").is_none());
        assert!(CountryCode::parse("U1").is_none());
    }

    #[test]
    fn only_regulatory_country_source_is_authoritative() {
        assert!(CountrySource::Regulatory.is_authoritative());
        assert!(!CountrySource::AccessPoint.is_authoritative());
        assert!(!CountrySource::UserRegion.is_authoritative());
        assert!(!CountrySource::Locale.is_authoritative());
    }

    #[test]
    fn network_ref_debug_is_redacted() {
        let net = NetworkRef(NetworkRefInner::Opaque("MySecretNetworkName".to_owned()));
        let rendered = format!("{net:?}");
        assert!(!rendered.contains("MySecretNetworkName"), "{rendered}");
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn password_outcome_equality_ignores_the_secret() {
        assert_eq!(
            PasswordOutcome::Found(Secret::new("a")),
            PasswordOutcome::Found(Secret::new("b"))
        );
        assert_ne!(
            PasswordOutcome::Found(Secret::new("a")),
            PasswordOutcome::NotStored
        );
        assert_eq!(PasswordOutcome::NotRequired, PasswordOutcome::NotRequired);
    }

    #[test]
    fn only_personal_networks_can_carry_a_passphrase() {
        assert!(SecurityKind::Personal.can_carry_passphrase());
        assert!(!SecurityKind::Open.can_carry_passphrase());
        assert!(!SecurityKind::Enterprise.can_carry_passphrase());
        assert!(!SecurityKind::UnsupportedSecurity.can_carry_passphrase());
        assert!(!SecurityKind::Unknown.can_carry_passphrase());
    }

    #[test]
    fn credential_validation_matches_the_t3_serializer_contract() {
        assert!(is_usable_wifi_credential("12345678"));
        assert!(is_usable_wifi_credential(&"x".repeat(63)));
        let psk = "0DC0D6EB90555ED6419756B9A15EC3E3209B63DF707DD508D14581F8982721AF";
        assert_eq!(psk.len(), 64);
        assert!(is_usable_wifi_credential(psk));
        assert!(is_usable_wifi_credential(&psk.to_lowercase()));

        assert!(!is_usable_wifi_credential(""));
        assert!(!is_usable_wifi_credential("1234567"));
        assert!(!is_usable_wifi_credential(&"z".repeat(64)));
        assert!(!is_usable_wifi_credential(&"a".repeat(65)));

        let turkish = "şifreçğü";
        assert_eq!(turkish.chars().count(), 8);
        assert_eq!(turkish.len(), 12);
        assert!(is_usable_wifi_credential(turkish));

        assert!(is_usable_wifi_credential("çğüöşia"));
        assert!(!is_usable_wifi_credential("abcdefg"));
    }
}
