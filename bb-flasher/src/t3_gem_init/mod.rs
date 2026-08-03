//! Safe serializer for the T3 GemStone first-boot file, `config.ini`.
//!
//! # Why this is not an INI writer
//!
//! The consumer is `gem-first-boot` in the T3 SDK, and it **`source`s** the file. Every line is
//! therefore shell code executed as root on the board's first boot. Two consequences shape this
//! module:
//!
//! * **Keys are a closed set.** [`Key`] is a compile-time whitelist and no public API accepts a key
//!   name, so no user input can ever become a variable name — or a command.
//! * **Values go through exactly one quoting function.** [`shell::quote`] is the only way a value
//!   becomes file text.
//!
//! # Why the field set is smaller than `gem-imager`'s
//!
//! `instruction.md` §10.1 restricts the file to what the current consumer actually reads.
//! `cryptsetup`, `diskpasswd`, `writeimagetommc`, the USB gadget toggles and the SSH options are
//! **not** written: `gem-first-boot` does not read them, so offering them would be a UI that claims
//! to configure something it does not.
//!
//! # Known SDK defect
//!
//! `gem-first-boot` reads `vncpassword` but its cleanup pass deletes `vncpasswd=` — the names do
//! not match, so the VNC secret survives on the boot partition after first boot. That is an SDK
//! bug, not something this crate can fix, and [`T3GemInitConfig::vnc_secret_survives_first_boot`]
//! exists so the UI can say so out loud instead of hiding it.

mod crypt;
mod secret;
mod shell;

pub use secret::Secret;

use secret::DerivedSecret;
use zeroize::Zeroizing;

/// File name the consumer looks for on the FAT boot partition.
pub const CONFIG_FILE_NAME: &str = "config.ini";

/// Byte length of a WPA PSK written as hex.
pub const WPA_PSK_HEX_LEN: usize = 64;

/// Shortest WPA passphrase, per IEEE 802.11i.
pub const WPA_PASSPHRASE_MIN_LEN: usize = 8;

/// Longest WPA passphrase, per IEEE 802.11i.
pub const WPA_PASSPHRASE_MAX_LEN: usize = 63;

/// The legacy VNC protocol only carries eight password bytes.
pub const VNC_PASSWORD_MAX_LEN: usize = 8;

/// Longest single hostname label (RFC 1123 §2.1).
const HOSTNAME_LABEL_MAX_LEN: usize = 63;

/// Longest fully-qualified hostname (RFC 1123 §2.1).
const HOSTNAME_MAX_LEN: usize = 253;

/// Longest SSID, per IEEE 802.11.
const SSID_MAX_LEN: usize = 32;

/// Keyboard layouts the imager offers.
///
/// The consumer writes this straight into `XKBLAYOUT`, so an unrecognised value produces a board
/// with an unusable keyboard. Only members of this list are accepted; it is kept sorted so callers
/// can binary-search it.
pub const KEYBOARD_LAYOUTS: &[&str] = &[
    "af", "al", "am", "ara", "at", "au", "az", "ba", "bd", "be", "bg", "br", "brai", "bt", "bw",
    "by", "ca", "cd", "ch", "cm", "cn", "cz", "de", "dk", "dz", "ee", "epo", "es", "et", "fi",
    "fo", "fr", "gb", "ge", "gh", "gn", "gr", "hr", "hu", "id", "ie", "il", "in", "iq", "ir", "is",
    "it", "jp", "jv", "ke", "kg", "kh", "kr", "kz", "la", "latam", "lk", "lt", "lv", "ma", "mao",
    "md", "me", "mk", "ml", "mm", "mn", "mt", "mv", "my", "ng", "nl", "no", "np", "ph", "pk", "pl",
    "pt", "ro", "rs", "ru", "se", "si", "sk", "sn", "sy", "tg", "th", "tj", "tm", "tr", "tw", "tz",
    "ua", "us", "uz", "vn", "za",
];

/// Everything that can go wrong turning a customization into `config.ini`.
///
/// No variant carries a secret: these strings reach logs and error dialogs, and a
/// "bad password: ..." message would defeat the redaction in [`Secret`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum T3GemInitError {
    #[error(
        "{field} contains a control character (0x{byte:02x}) and cannot be written to config.ini"
    )]
    ControlCharacter { field: &'static str, byte: u8 },

    #[error("hostname is not a valid RFC 1123 host name")]
    InvalidHostname,

    #[error("Wi-Fi country must be two upper-case ASCII letters")]
    InvalidWifiCountry,

    #[error("SSID must be 1 to {SSID_MAX_LEN} bytes")]
    InvalidSsid,

    #[error("\"{0}\" is not a time zone this application offers")]
    UnknownTimezone(String),

    #[error("\"{0}\" is not a keyboard layout this application offers")]
    UnknownKeyboardLayout(String),

    #[error(
        "Wi-Fi password must be a {WPA_PASSPHRASE_MIN_LEN}-{WPA_PASSPHRASE_MAX_LEN} character \
         passphrase or a {WPA_PSK_HEX_LEN}-digit hexadecimal PSK"
    )]
    WifiPassphraseLength,

    #[error("VNC passwords are limited to {VNC_PASSWORD_MAX_LEN} bytes, but this one is {len}")]
    VncPasswordTooLong { len: usize },

    #[error("account password cannot be empty")]
    EmptyPassword,

    #[error("failed to derive a password hash")]
    PasswordHash,

    #[error("the operating system random number generator is unavailable")]
    Csprng,
}

/// The closed set of keys `gem-first-boot` reads.
///
/// This enum is private on purpose: it is the mechanism that makes "the user cannot supply a key
/// name" a property of the type system rather than a review comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    Firstboot,
    Hostname,
    UserPasswd,
    WifiName,
    WifiPasswd,
    WifiCountry,
    Timezone,
    KeyboardLayout,
    Vnc,
    VncPassword,
}

impl Key {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Firstboot => "firstboot",
            Self::Hostname => "hostname",
            Self::UserPasswd => "userpasswd",
            Self::WifiName => "wifiname",
            Self::WifiPasswd => "wifipasswd",
            Self::WifiCountry => "wificountry",
            Self::Timezone => "timezone",
            Self::KeyboardLayout => "keyboardlayout",
            Self::Vnc => "vnc",
            Self::VncPassword => "vncpassword",
        }
    }
}

/// An RFC 1123 host name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Hostname(String);

impl Hostname {
    /// Validate a host name.
    ///
    /// RFC 1123 §2.1 relaxes RFC 952 to allow a leading digit, which is why `3gem` is legal. A
    /// leading or trailing hyphen is not, and neither is an empty label.
    pub fn parse(value: &str) -> Result<Self, T3GemInitError> {
        if value.is_empty() || value.len() > HOSTNAME_MAX_LEN {
            return Err(T3GemInitError::InvalidHostname);
        }

        let valid = value.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= HOSTNAME_LABEL_MAX_LEN
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });

        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(T3GemInitError::InvalidHostname)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An ISO 3166-1 alpha-2 regulatory domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WifiCountry([u8; 2]);

impl WifiCountry {
    /// Validate a country code.
    ///
    /// The value ends up in the kernel's 802.11 regulatory domain, which accepts exactly two
    /// upper-case ASCII letters. Lower case input is upper-cased rather than rejected, because that
    /// is a typing convention rather than a different value.
    pub fn parse(value: &str) -> Result<Self, T3GemInitError> {
        let bytes = value.as_bytes();
        if bytes.len() != 2 || !bytes.iter().all(u8::is_ascii_alphabetic) {
            return Err(T3GemInitError::InvalidWifiCountry);
        }

        Ok(Self([
            bytes[0].to_ascii_uppercase(),
            bytes[1].to_ascii_uppercase(),
        ]))
    }

    pub fn as_str(&self) -> &str {
        // Only ASCII letters can reach the constructor.
        std::str::from_utf8(&self.0).expect("WifiCountry is ASCII by construction")
    }
}

/// An IANA time zone name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Timezone(String);

impl Timezone {
    /// Validate a time zone against the IANA database compiled into the application.
    ///
    /// This is the "only from the application's validated list" rule of `instruction.md` §10.2: the
    /// same database backs the GUI's time zone picker, so a value that passes here is one the user
    /// could have selected.
    pub fn parse(value: &str) -> Result<Self, T3GemInitError> {
        value
            .parse::<chrono_tz::Tz>()
            .map(|tz| Self(tz.name().to_owned()))
            .map_err(|_| T3GemInitError::UnknownTimezone(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An X11 keyboard layout from [`KEYBOARD_LAYOUTS`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyboardLayout(&'static str);

impl KeyboardLayout {
    pub fn parse(value: &str) -> Result<Self, T3GemInitError> {
        KEYBOARD_LAYOUTS
            .binary_search(&value)
            .map(|i| Self(KEYBOARD_LAYOUTS[i]))
            .map_err(|_| T3GemInitError::UnknownKeyboardLayout(value.to_owned()))
    }

    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

/// A wireless network name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ssid(String);

impl Ssid {
    /// Validate an SSID.
    ///
    /// The 802.11 limit is 32 **bytes**, not characters — a Turkish SSID reaches it sooner than an
    /// ASCII one, and truncating it would join the wrong network.
    pub fn parse(value: &str) -> Result<Self, T3GemInitError> {
        if value.is_empty() || value.len() > SSID_MAX_LEN {
            return Err(T3GemInitError::InvalidSsid);
        }

        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Wireless credentials, resolved to a PSK at serialization time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiSettings {
    pub ssid: Ssid,
    /// Either an 8..=63 byte passphrase or a 64-digit hex PSK; which one is decided by length.
    pub password: Secret,
    pub country: WifiCountry,
}

/// VNC settings, only offered on desktop images.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VncSettings {
    pub password: Secret,
}

/// A validated T3 first-boot customization.
///
/// Values are validated when they are set, so an instance of this type can always be serialized
/// except for the two derivations that still depend on input length (Wi-Fi password and VNC
/// password), which are checked in [`Self::serialize`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct T3GemInitConfig {
    hostname: Option<Hostname>,
    user_password: Option<Secret>,
    wifi: Option<WifiSettings>,
    timezone: Option<Timezone>,
    keyboard_layout: Option<KeyboardLayout>,
    vnc: Option<VncSettings>,
}

impl T3GemInitConfig {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_hostname(mut self, hostname: Option<Hostname>) -> Self {
        self.hostname = hostname;
        self
    }

    /// Set the account password. An empty password is rejected at serialization time rather than
    /// silently producing an account with no password.
    #[must_use]
    pub fn with_user_password(mut self, password: Option<Secret>) -> Self {
        self.user_password = password;
        self
    }

    #[must_use]
    pub fn with_wifi(mut self, wifi: Option<WifiSettings>) -> Self {
        self.wifi = wifi;
        self
    }

    #[must_use]
    pub fn with_timezone(mut self, timezone: Option<Timezone>) -> Self {
        self.timezone = timezone;
        self
    }

    #[must_use]
    pub fn with_keyboard_layout(mut self, layout: Option<KeyboardLayout>) -> Self {
        self.keyboard_layout = layout;
        self
    }

    #[must_use]
    pub fn with_vnc(mut self, vnc: Option<VncSettings>) -> Self {
        self.vnc = vnc;
        self
    }

    /// Whether this configuration leaves a secret on the boot partition after first boot.
    ///
    /// `gem-first-boot` reads `vncpassword` but scrubs `vncpasswd=`, so the VNC line survives. The
    /// UI is required to surface this rather than quietly ship it.
    pub const fn vnc_secret_survives_first_boot(&self) -> bool {
        self.vnc.is_some()
    }

    /// Render the file.
    ///
    /// `firstboot=1` is always the first line: it is the guard the consumer tests before doing
    /// anything at all, and a file without it is inert.
    pub fn serialize(&self) -> Result<Zeroizing<Vec<u8>>, T3GemInitError> {
        let mut out = Writer::default();

        // Unquoted on purpose: this is a literal the consumer compares against `1`, not user data.
        out.raw(Key::Firstboot, "1");

        if let Some(hostname) = &self.hostname {
            out.quoted(Key::Hostname, hostname.as_str())?;
        }

        if let Some(password) = &self.user_password {
            if password.is_empty() {
                return Err(T3GemInitError::EmptyPassword);
            }
            let hash = crypt::sha512_crypt_os_salt(password)?;
            out.quoted(Key::UserPasswd, &hash)?;
        }

        if let Some(wifi) = &self.wifi {
            let psk = derive_wifi_psk(wifi)?;
            out.quoted(Key::WifiName, wifi.ssid.as_str())?;
            out.quoted(Key::WifiPasswd, &psk)?;
            out.quoted(Key::WifiCountry, wifi.country.as_str())?;
        }

        if let Some(timezone) = &self.timezone {
            out.quoted(Key::Timezone, timezone.as_str())?;
        }

        if let Some(layout) = &self.keyboard_layout {
            out.quoted(Key::KeyboardLayout, layout.as_str())?;
        }

        if let Some(vnc) = &self.vnc {
            let obfuscated = crypt::vnc_obfuscate(&vnc.password)?;
            out.raw(Key::Vnc, "1");
            out.quoted(Key::VncPassword, &obfuscated)?;
        }

        Ok(out.finish())
    }
}

/// Resolve a Wi-Fi password to a PSK.
///
/// The length decides the interpretation, which is how every WPA supplicant does it: exactly 64 hex
/// digits is already a PSK, and 8..=63 characters is a passphrase to run through PBKDF2. Anything
/// else is neither, and guessing would produce a card that silently fails to join the network.
fn derive_wifi_psk(wifi: &WifiSettings) -> Result<DerivedSecret, T3GemInitError> {
    match wifi.password.len() {
        WPA_PSK_HEX_LEN => crypt::normalize_psk_hex(&wifi.password),
        WPA_PASSPHRASE_MIN_LEN..=WPA_PASSPHRASE_MAX_LEN => {
            crypt::wpa_psk(wifi.ssid.as_str(), &wifi.password)
        }
        _ => Err(T3GemInitError::WifiPassphraseLength),
    }
}

/// Accumulates `config.ini` lines. The only way to add one is through a [`Key`].
#[derive(Default)]
struct Writer(Zeroizing<Vec<u8>>);

impl Writer {
    /// Write a value that is a compile-time literal, not user input.
    fn raw(&mut self, key: Key, value: &'static str) {
        self.0.extend_from_slice(key.as_str().as_bytes());
        self.0.push(b'=');
        self.0.extend_from_slice(value.as_bytes());
        self.0.push(b'\n');
    }

    /// Write a value through the shell literal serializer.
    fn quoted(&mut self, key: Key, value: &str) -> Result<(), T3GemInitError> {
        let quoted = Zeroizing::new(shell::quote(key.as_str(), value)?);
        self.0.extend_from_slice(key.as_str().as_bytes());
        self.0.push(b'=');
        self.0.extend_from_slice(quoted.as_bytes());
        self.0.push(b'\n');

        Ok(())
    }

    fn finish(self) -> Zeroizing<Vec<u8>> {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(config: &T3GemInitConfig) -> String {
        String::from_utf8(config.serialize().unwrap().to_vec()).unwrap()
    }

    #[test]
    fn an_empty_configuration_is_still_a_valid_guard_file() {
        assert_eq!(rendered(&T3GemInitConfig::new()), "firstboot=1\n");
    }

    #[test]
    fn firstboot_is_always_the_first_line() {
        let config = T3GemInitConfig::new()
            .with_hostname(Some(Hostname::parse("gemstone").unwrap()))
            .with_timezone(Some(Timezone::parse("Europe/Istanbul").unwrap()));

        assert!(rendered(&config).starts_with("firstboot=1\n"));
    }

    #[test]
    fn every_supported_field_reaches_the_expected_line() {
        let config = T3GemInitConfig::new()
            .with_hostname(Some(Hostname::parse("t3-gemstone").unwrap()))
            .with_wifi(Some(WifiSettings {
                ssid: Ssid::parse("Ağ-Çekirdek").unwrap(),
                password: Secret::new("parola1234"),
                country: WifiCountry::parse("tr").unwrap(),
            }))
            .with_timezone(Some(Timezone::parse("Europe/Istanbul").unwrap()))
            .with_keyboard_layout(Some(KeyboardLayout::parse("tr").unwrap()));

        let out = rendered(&config);
        assert!(out.contains("hostname='t3-gemstone'\n"));
        assert!(out.contains("wifiname='Ağ-Çekirdek'\n"));
        assert!(out.contains("wificountry='TR'\n"));
        assert!(out.contains("timezone='Europe/Istanbul'\n"));
        assert!(out.contains("keyboardlayout='tr'\n"));
        // The passphrase itself must never appear; only the derived PSK does.
        assert!(!out.contains("parola1234"));
    }

    /// Keys the current `gem-first-boot` does not read must not reach the file
    /// (`instruction.md` §10.1). There is no API to set them, so this asserts the whole surface.
    #[test]
    fn unsupported_keys_can_never_appear() {
        let config = T3GemInitConfig::new()
            .with_hostname(Some(Hostname::parse("gemstone").unwrap()))
            .with_user_password(Some(Secret::new("s3cret")))
            .with_vnc(Some(VncSettings {
                password: Secret::new("1234"),
            }));

        let out = rendered(&config);
        for forbidden in [
            "cryptsetup",
            "diskpasswd",
            "writeimagetommc",
            "storagegadget",
            "ethernetgadget",
            "serialgadgets",
            "ssh",
            "user_authorized_key",
        ] {
            assert!(
                !out.contains(forbidden),
                "{forbidden} leaked into config.ini"
            );
        }
    }

    /// The end-to-end injection test: a hostile value in every text field, and the file still has
    /// exactly one line per key with the payload trapped inside a literal.
    #[test]
    fn injection_payloads_cannot_create_lines_or_keys() {
        let payload = "a$(id)`id`\\'\"; export EVIL=1; #";
        let config = T3GemInitConfig::new()
            .with_wifi(Some(WifiSettings {
                ssid: Ssid::parse(payload).unwrap(),
                password: Secret::new("passphrase"),
                country: WifiCountry::parse("TR").unwrap(),
            }))
            .with_user_password(Some(Secret::new(payload)));

        let out = rendered(&config);
        let parsed = parse_like_shell(&out);
        let keys: Vec<&str> = parsed.iter().map(|(k, _)| k.as_str()).collect();

        // The payload creates no extra assignment and no extra line.
        assert_eq!(
            keys,
            [
                "firstboot",
                "userpasswd",
                "wifiname",
                "wifipasswd",
                "wificountry"
            ]
        );
        // It survives as *data*: the SSID reads back exactly as typed, metacharacters and all.
        assert_eq!(parsed[2].1, payload);
        // And `EVIL` is not a variable the file defines — the literal text is inside the SSID
        // value, which is the whole point of quoting it.
        assert!(!keys.contains(&"EVIL"));
    }

    /// Round-trip through an isolated parser that reads the file the way `source` would.
    ///
    /// `instruction.md` §10.5 asks for a shell round-trip in which only the expected variables
    /// appear and no command runs. Spawning a real shell would make the test depend on the host
    /// having one and on it never executing what it reads, so the parser is reimplemented here:
    /// it understands exactly `key=value` and `key='literal'` with the `'\''` splice, and treats
    /// anything else as a parse failure. If the serializer ever emitted something a shell would
    /// *execute* rather than assign, this would not parse.
    fn parse_like_shell(content: &str) -> Vec<(String, String)> {
        content
            .lines()
            .map(|line| {
                let (key, raw) = line.split_once('=').expect("every line is an assignment");
                assert!(
                    key.bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()),
                    "{key} is not a plain variable name"
                );

                let value = if let Some(inner) = raw.strip_prefix('\'') {
                    let inner = inner.strip_suffix('\'').expect("literal is closed");
                    // The only escape a single-quoted literal can contain.
                    let unquoted = inner.replace("'\\''", "'");
                    assert!(
                        !unquoted.contains('\'') || inner.contains("'\\''"),
                        "unescaped quote would have ended the literal early"
                    );
                    unquoted
                } else {
                    // Unquoted values are only ever compile-time literals like `1`.
                    assert!(
                        raw.bytes().all(|b| b.is_ascii_digit()),
                        "unquoted value {raw} is not a bare number"
                    );
                    raw.to_owned()
                };

                (key.to_owned(), value)
            })
            .collect()
    }

    /// Every value the user typed comes back out of the file byte-identical.
    #[test]
    fn values_round_trip_through_the_shell_parser() {
        let config = T3GemInitConfig::new()
            .with_hostname(Some(Hostname::parse("t3-gemstone").unwrap()))
            .with_wifi(Some(WifiSettings {
                ssid: Ssid::parse("Ağ'ı $HOME `id`").unwrap(),
                password: Secret::new("parola1234"),
                country: WifiCountry::parse("tr").unwrap(),
            }))
            .with_timezone(Some(Timezone::parse("Europe/Istanbul").unwrap()));

        let parsed = parse_like_shell(&rendered(&config));
        let get = |k: &str| {
            parsed
                .iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.as_str())
                .unwrap()
        };

        assert_eq!(get("firstboot"), "1");
        assert_eq!(get("hostname"), "t3-gemstone");
        assert_eq!(get("wifiname"), "Ağ'ı $HOME `id`");
        assert_eq!(get("wificountry"), "TR");
        assert_eq!(get("timezone"), "Europe/Istanbul");
    }

    /// A newline is the one payload quoting cannot make safe for a line-oriented consumer, so the
    /// whole serialization fails rather than writing a mangled file.
    #[test]
    fn a_newline_in_a_value_fails_the_whole_file() {
        let err = T3GemInitConfig::new()
            .with_wifi(Some(WifiSettings {
                ssid: Ssid::parse("evil\nvnc=1").unwrap(),
                password: Secret::new("passphrase"),
                country: WifiCountry::parse("TR").unwrap(),
            }))
            .serialize()
            .unwrap_err();

        assert!(matches!(err, T3GemInitError::ControlCharacter { .. }));
    }

    #[test]
    fn hostnames_follow_rfc_1123() {
        let longest_label = "a".repeat(63);
        for good in [
            "gemstone",
            "t3-gem-o1",
            "3gem",
            "a",
            "a.b.c",
            longest_label.as_str(),
        ] {
            assert!(Hostname::parse(good).is_ok(), "{good} should be valid");
        }

        let too_long_label = "a".repeat(64);
        for bad in [
            "",
            "-gemstone",
            "gemstone-",
            "gem stone",
            "gem_stone",
            "gemstone.",
            "ünikod",
            too_long_label.as_str(),
        ] {
            assert!(Hostname::parse(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn wifi_country_is_two_upper_case_ascii_letters() {
        assert_eq!(WifiCountry::parse("tr").unwrap().as_str(), "TR");
        assert_eq!(WifiCountry::parse("DE").unwrap().as_str(), "DE");
        for bad in ["T", "TUR", "T1", "", "tü"] {
            assert!(WifiCountry::parse(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn ssid_length_is_measured_in_bytes() {
        assert!(Ssid::parse(&"a".repeat(32)).is_ok());
        assert!(Ssid::parse(&"a".repeat(33)).is_err());
        assert!(Ssid::parse("").is_err());
        // 16 two-byte characters is exactly the limit; 17 is over it even though it is shorter to
        // read.
        assert!(Ssid::parse(&"ç".repeat(16)).is_ok());
        assert!(Ssid::parse(&"ç".repeat(17)).is_err());
    }

    #[test]
    fn timezone_and_keymap_only_accept_offered_values() {
        assert_eq!(
            Timezone::parse("Europe/Istanbul").unwrap().as_str(),
            "Europe/Istanbul"
        );
        assert!(Timezone::parse("Europe/Gemstone").is_err());
        assert!(Timezone::parse("../../etc/localtime").is_err());

        assert_eq!(KeyboardLayout::parse("tr").unwrap().as_str(), "tr");
        assert!(KeyboardLayout::parse("TR").is_err());
        assert!(KeyboardLayout::parse("tr; id").is_err());
    }

    #[test]
    fn keyboard_layout_list_is_sorted_for_binary_search() {
        assert!(KEYBOARD_LAYOUTS.is_sorted());
    }

    #[test]
    fn wifi_password_length_selects_the_derivation() {
        let ssid = Ssid::parse("ThisIsASSID").unwrap();
        let country = WifiCountry::parse("TR").unwrap();

        // A passphrase is run through PBKDF2 with the SSID as salt.
        let passphrase = derive_wifi_psk(&WifiSettings {
            ssid: ssid.clone(),
            password: Secret::new("ThisIsAPassword"),
            country,
        })
        .unwrap();
        assert_eq!(
            *passphrase,
            "0dc0d6eb90555ed6419756b9a15ec3e3209b63df707dd508d14581f8982721af"
        );

        // A 64-hex PSK is taken as-is, not hashed again.
        let psk = derive_wifi_psk(&WifiSettings {
            ssid: ssid.clone(),
            password: Secret::new(passphrase.to_uppercase()),
            country,
        })
        .unwrap();
        assert_eq!(*psk, *passphrase);

        // Five characters is neither a legal passphrase nor a PSK.
        assert!(matches!(
            derive_wifi_psk(&WifiSettings {
                ssid,
                password: Secret::new("short"),
                country,
            }),
            Err(T3GemInitError::WifiPassphraseLength)
        ));
    }

    #[test]
    fn an_empty_account_password_is_rejected() {
        assert!(matches!(
            T3GemInitConfig::new()
                .with_user_password(Some(Secret::default()))
                .serialize(),
            Err(T3GemInitError::EmptyPassword)
        ));
    }

    #[test]
    fn the_account_password_is_written_only_as_a_crypt_hash() {
        let config = T3GemInitConfig::new().with_user_password(Some(Secret::new("gemstone")));
        let out = rendered(&config);

        assert!(!out.contains("gemstone"));
        let line = out.lines().find(|l| l.starts_with("userpasswd=")).unwrap();
        let hash = line
            .trim_start_matches("userpasswd='")
            .trim_end_matches('\'');
        assert!(hash.starts_with("$6$"));
        sha_crypt::sha512_check("gemstone", hash).expect("the board can verify this hash");
    }

    #[test]
    fn vnc_is_only_written_when_enabled_and_is_flagged_as_a_surviving_secret() {
        let off = T3GemInitConfig::new();
        assert!(!off.vnc_secret_survives_first_boot());
        assert!(!rendered(&off).contains("vnc"));

        let on = T3GemInitConfig::new().with_vnc(Some(VncSettings {
            password: Secret::new("1234"),
        }));
        assert!(on.vnc_secret_survives_first_boot());
        let out = rendered(&on);
        assert!(out.contains("vnc=1\n"));
        assert!(out.contains("vncpassword='ee5b0e48c8fe9771'\n"));
        assert!(!out.contains("'1234'"));
    }
}
