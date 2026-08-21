//! Platform-neutral description of the host's current Wi-Fi network.
//!
//! These types deliberately model *partial* knowledge: a missing password must never discard a
//! perfectly good SSID or country (§5.3), so discovery and password retrieval are separate results
//! rather than one `Result<AllFields, _>`.

use gem_helper::secret::Secret;

/// An opaque handle to the network that [`crate::detect_current_wifi`] found, used to ask for its
/// saved password in a second step.
///
/// Opaque on purpose (§10.2): the caller must not parse it, show it, or log it. Each platform puts
/// the identity its password lookup needs inside, and none of that is meaningful or safe outside
/// the backend that produced it.
#[derive(Clone, PartialEq, Eq)]
pub struct NetworkRef(pub(crate) NetworkRefInner);

/// The platform payload behind a [`NetworkRef`]. `pub(crate)` so only the backends construct it.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum NetworkRefInner {
    #[cfg(target_os = "linux")]
    /// NetworkManager `Settings.Connection` object path of the active connection.
    NetworkManager { connection_path: String },
    #[cfg(target_os = "windows")]
    Windows {
        interface_guid: String,
        profile_name: String,
    },
    #[cfg(target_os = "macos")]
    MacOs { ssid: String },
    /// Keeps the enum inhabited on platforms with no backend compiled in, and lets the shared tests
    /// build a value without a live OS.
    #[cfg_attr(
        any(target_os = "linux", target_os = "windows", target_os = "macos"),
        allow(dead_code)
    )]
    Opaque(String),
}

impl std::fmt::Debug for NetworkRef {
    /// Redacted: a `NetworkRef` can embed a network name, and that must not reach a log or a panic
    /// dump (§10.2). The variant is useful for debugging; its contents are not.
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

/// The result of discovering the current network. Everything here is safe to display *except* that
/// the SSID must be applied to a form field, never logged as identifying data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedWifi {
    pub network: NetworkRef,
    pub ssid: DetectedSsid,
    pub security: SecurityKind,
    pub country: Option<CountryHint>,
}

/// A Wi-Fi SSID is a byte string, not text. Most are UTF-8, but the standard permits arbitrary
/// bytes, and a lossy conversion would silently corrupt a name (§8.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DetectedSsid {
    Utf8(String),
    /// The SSID exists but is not valid UTF-8, so it cannot be shown in the text-based form. The
    /// caller keeps the network selected and asks the user to type the name.
    UnsupportedEncoding,
}

impl DetectedSsid {
    /// The name to place in the SSID field, if it can be represented as text.
    pub fn as_utf8(&self) -> Option<&str> {
        match self {
            Self::Utf8(s) => Some(s),
            Self::UnsupportedEncoding => None,
        }
    }

    /// Build from raw SSID bytes, classifying the encoding rather than forcing it.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        match std::str::from_utf8(bytes) {
            Ok(s) => Self::Utf8(s.to_owned()),
            Err(_) => Self::UnsupportedEncoding,
        }
    }
}

/// How the current network is secured. This decides whether asking for a password even makes sense:
/// only [`SecurityKind::Personal`] carries a single portable passphrase the imager can reuse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecurityKind {
    /// No password (§8.4). Filling one in would be wrong, not missing.
    Open,
    /// WPA/WPA2-PSK or WPA3-SAE: a single passphrase the user could carry to the board.
    Personal,
    /// 802.1X / EAP / certificate: no single portable secret exists, so import is not attempted.
    Enterprise,
    /// A secured network whose credential the imager cannot carry over: static WEP, and any other
    /// scheme that is neither open nor a WPA-family passphrase (§8.4). Kept distinct from
    /// [`SecurityKind::Open`] so the UI never says "no password needed" about a secured network.
    UnsupportedSecurity,
    /// Could not be classified. Discovery still succeeds; password retrieval will report why.
    Unknown,
}

impl SecurityKind {
    /// Whether a single portable passphrase can exist for this network at all.
    ///
    /// Backends check this *before* asking the OS for a secret: there is no point prompting the
    /// user, or the platform, for a credential that could never be applied to the board.
    pub fn can_carry_passphrase(self) -> bool {
        matches!(self, Self::Personal)
    }
}

/// Shortest WPA passphrase, per IEEE 802.11i.
const WPA_PASSPHRASE_MIN_LEN: usize = 8;
/// Longest WPA passphrase, per IEEE 802.11i.
const WPA_PASSPHRASE_MAX_LEN: usize = 63;
/// Length of a WPA PSK written as hex.
const WPA_PSK_HEX_LEN: usize = 64;

/// Whether a retrieved credential is shaped like something the T3 image can actually use.
///
/// A value coming from the platform is not automatically valid (§3.1): it still has to satisfy the
/// contract the T3 serializer enforces — an 8..=63 byte passphrase (kept as text so WPA3/SAE works)
/// or exactly 64 hexadecimal digits for a ready-made WPA2 PSK.
///
/// Takes the plaintext by reference and returns only a bool, so no secret escapes.
pub fn is_usable_wifi_credential(value: &str) -> bool {
    match value.len() {
        WPA_PSK_HEX_LEN => value.bytes().all(|b| b.is_ascii_hexdigit()),
        WPA_PASSPHRASE_MIN_LEN..=WPA_PASSPHRASE_MAX_LEN => true,
        _ => false,
    }
}

/// The outcome of trying to read the saved password. Every non-`Found` variant is a normal result
/// the UI states plainly, not an error (§5.3): the manual field stays as the first-class fallback.
#[derive(Debug)]
pub enum PasswordOutcome {
    /// A usable Personal passphrase was retrieved.
    Found(Secret),
    /// The network is open, so no password is needed.
    NotRequired,
    /// The OS has no saved password for this network (e.g. NetworkManager `psk-flags = not-saved`).
    NotStored,
    /// The OS refused without an interactive prompt (missing privilege, DACL, policy).
    PermissionDenied,
    /// An interactive OS prompt was shown and the user declined (macOS Keychain "Deny").
    UserDenied,
    /// An interactive OS prompt was shown and the user dismissed it.
    UserCancelled,
    /// The security type has no single portable passphrase (Enterprise, WEP, OWE).
    UnsupportedSecurity,
    /// A saved credential exists but could not be turned into plaintext (e.g. still encrypted).
    Unavailable,
}

impl PartialEq for PasswordOutcome {
    /// Compares by variant only; the wrapped [`Secret`] is never inspected for equality here so a
    /// password cannot leak through a failing test assertion. Two `Found` values are considered
    /// equal regardless of contents, which is what the outcome-classification tests check.
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

/// A best-effort two-letter country code plus where it came from (§9). The source drives the UI
/// hint: anything other than [`CountrySource::Regulatory`] is a guess the user should confirm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryHint {
    pub code: CountryCode,
    pub source: CountrySource,
}

/// Where a [`CountryHint`] came from, most to least authoritative.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CountrySource {
    /// The wireless regulatory domain the radio is actually operating under.
    Regulatory,
    /// A country element advertised by the access point.
    AccessPoint,
    /// The user's configured system region.
    UserRegion,
    /// The system locale, the weakest signal.
    Locale,
}

impl CountrySource {
    /// Whether this source is authoritative enough to present without a "we guessed" caveat.
    pub fn is_authoritative(self) -> bool {
        matches!(self, Self::Regulatory)
    }
}

/// A validated ISO 3166-1 alpha-2 country code: exactly two ASCII letters, upper-cased.
///
/// Validated at construction so a bad locale string (`"C"`, `"en_US.UTF-8"`, the regulatory
/// "world" placeholder `"00"`) can never reach the T3 serializer, which expects a clean code.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CountryCode([u8; 2]);

impl CountryCode {
    /// Parse and normalise a country code, rejecting anything that is not two ASCII letters.
    ///
    /// `"00"` — the "world regulatory domain" placeholder — is rejected so it is treated as
    /// unknown rather than applied as if it were a real country (§9).
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
        // Safe: the buffer is always two ASCII letters by construction.
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
        // A lone 0xFF is never valid UTF-8: must be flagged, not lossily converted.
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
        // World regulatory placeholder and malformed locales are not countries.
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
        // A failing assertion on outcomes must never print or compare the passphrase itself.
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
        // 8..=63 bytes is a passphrase; exactly 64 hex digits is a ready-made PSK.
        assert!(is_usable_wifi_credential("12345678"));
        assert!(is_usable_wifi_credential(&"x".repeat(63)));
        let psk = "0DC0D6EB90555ED6419756B9A15EC3E3209B63DF707DD508D14581F8982721AF";
        assert_eq!(psk.len(), 64);
        assert!(is_usable_wifi_credential(psk));
        assert!(is_usable_wifi_credential(&psk.to_lowercase()));

        // Too short, and the 64-byte length only passes when it is really hex.
        assert!(!is_usable_wifi_credential(""));
        assert!(!is_usable_wifi_credential("1234567"));
        assert!(!is_usable_wifi_credential(&"z".repeat(64)));
        // 64 < len is out of range even for a hex-looking value.
        assert!(!is_usable_wifi_credential(&"a".repeat(65)));

        // Measured in bytes like the serializer: 8 characters but 12 bytes, so inside the range.
        let turkish = "şifreçğü";
        assert_eq!(turkish.chars().count(), 8);
        assert_eq!(turkish.len(), 12);
        assert!(is_usable_wifi_credential(turkish));

        // The boundary is in bytes both ways: 7 multi-byte characters is 12 bytes and passes, 7 ASCII does not.
        assert!(is_usable_wifi_credential("çğüöşia"));
        assert!(!is_usable_wifi_credential("abcdefg"));
    }
}
