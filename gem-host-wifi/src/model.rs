//! Platform-neutral description of the host's current Wi-Fi network.
//!
//! These types are the contract between the OS-specific backends in [`crate::pal`] and every
//! caller. They deliberately model *partial* knowledge: the research plan (§5.3) requires that a
//! missing password never discards a perfectly good SSID or country, so discovery and password
//! retrieval are separate results rather than one `Result<AllFields, _>`.

use gem_helper::secret::Secret;

/// An opaque handle to the network that [`crate::detect_current_wifi`] found, used to ask for its
/// saved password in a second step.
///
/// It is opaque on purpose (§10.2): the caller must not parse it, show it, or log it. Each platform
/// puts the identity its password lookup needs inside — a NetworkManager connection path, a Windows
/// interface GUID plus resolved profile name, or a macOS SSID plus Keychain query identity — and
/// none of that is meaningful or safe outside the backend that produced it.
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
    /// Keeps the enum inhabited and `match`-able on platforms with no backend compiled in, and lets
    /// the shared tests build a value without a live OS.
    #[cfg_attr(
        any(target_os = "linux", target_os = "windows", target_os = "macos"),
        allow(dead_code)
    )]
    Opaque(String),
}

impl std::fmt::Debug for NetworkRef {
    /// Redacted: a `NetworkRef` can embed a network name, and the plan forbids it reaching a log or
    /// a panic dump (§10.2). The variant is useful for debugging; its contents are not.
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
/// bytes, and the plan (§8.1) forbids a lossy conversion that would silently corrupt a name.
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
    /// Could not be classified. Discovery still succeeds; password retrieval will report why.
    Unknown,
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
/// hint: anything other than [`CountrySource::Regulatory`] is a guess the user should confirm, and
/// the field always stays editable.
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
    /// `"00"` — NetworkManager/`iw`'s "no specific country / world regulatory domain" — is rejected
    /// so it is treated as "unknown" rather than applied as if it were a real country (§9).
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
        assert_ne!(PasswordOutcome::Found(Secret::new("a")), PasswordOutcome::NotStored);
        assert_eq!(PasswordOutcome::NotRequired, PasswordOutcome::NotRequired);
    }
}
