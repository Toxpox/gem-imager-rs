
mod crypt;
mod secret;
mod shell;

pub use secret::Secret;

use secret::DerivedSecret;
use zeroize::Zeroizing;

pub const CONFIG_FILE_NAME: &str = "config.ini";

pub const WPA_PSK_HEX_LEN: usize = 64;

pub const WPA_PASSPHRASE_MIN_LEN: usize = 8;

pub const WPA_PASSPHRASE_MAX_LEN: usize = 63;

pub const VNC_PASSWORD_MAX_LEN: usize = 8;

const HOSTNAME_LABEL_MAX_LEN: usize = 63;

const HOSTNAME_MAX_LEN: usize = 253;

const SSID_MAX_LEN: usize = 32;

pub const KEYBOARD_LAYOUTS: &[&str] = &[
    "af", "al", "am", "ara", "at", "au", "az", "ba", "bd", "be", "bg", "br", "brai", "bt", "bw",
    "by", "ca", "cd", "ch", "cm", "cn", "cz", "de", "dk", "dz", "ee", "epo", "es", "et", "fi",
    "fo", "fr", "gb", "ge", "gh", "gn", "gr", "hr", "hu", "id", "ie", "il", "in", "iq", "ir", "is",
    "it", "jp", "jv", "ke", "kg", "kh", "kr", "kz", "la", "latam", "lk", "lt", "lv", "ma", "mao",
    "md", "me", "mk", "ml", "mm", "mn", "mt", "mv", "my", "ng", "nl", "no", "np", "ph", "pk", "pl",
    "pt", "ro", "rs", "ru", "se", "si", "sk", "sn", "sy", "tg", "th", "tj", "tm", "tr", "tw", "tz",
    "ua", "us", "uz", "vn", "za",
];

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

    #[error(
        "an SSID containing '/' cannot be configured, because the board's first-boot script uses \
         it as a file name"
    )]
    SsidUnsupportedByCurrentSdk,

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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Hostname(String);

impl Hostname {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WifiCountry([u8; 2]);

impl WifiCountry {
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
        std::str::from_utf8(&self.0).expect("WifiCountry is ASCII by construction")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Timezone(String);

impl Timezone {
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ssid(String);

impl Ssid {
    pub fn parse(value: &str) -> Result<Self, T3GemInitError> {
        if value.is_empty() || value.len() > SSID_MAX_LEN {
            return Err(T3GemInitError::InvalidSsid);
        }

        if value.contains('/') {
            return Err(T3GemInitError::SsidUnsupportedByCurrentSdk);
        }

        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiSettings {
    pub ssid: Ssid,
    pub password: Secret,
    pub country: WifiCountry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VncSettings {
    pub password: Secret,
}

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

    pub const fn vnc_secret_survives_first_boot(&self) -> bool {
        self.vnc.is_some()
    }

    pub fn serialize(&self) -> Result<Zeroizing<Vec<u8>>, T3GemInitError> {
        let mut out = Writer::default();

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
            let key = wifi_key(&wifi.password)?;
            out.quoted(Key::WifiName, &escape_for_keyfile(wifi.ssid.as_str()))?;
            out.quoted(Key::WifiPasswd, &escape_for_keyfile(&key))?;
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

fn wifi_key(password: &Secret) -> Result<DerivedSecret, T3GemInitError> {
    match password.len() {
        WPA_PSK_HEX_LEN => crypt::normalize_psk_hex(password),
        WPA_PASSPHRASE_MIN_LEN..=WPA_PASSPHRASE_MAX_LEN => {
            Ok(DerivedSecret::new(password.expose().to_owned()))
        }
        _ => Err(T3GemInitError::WifiPassphraseLength),
    }
}

fn escape_for_keyfile(value: &str) -> DerivedSecret {
    let mut out = String::with_capacity(value.len());
    let mut leading_whitespace = true;

    for c in value.chars() {
        match c {
            '\\' => {
                out.push_str(r"\\");
                leading_whitespace = false;
            }
            '\t' => out.push_str(r"\t"),
            ' ' if leading_whitespace => out.push_str(r"\s"),
            _ => {
                out.push(c);
                leading_whitespace = false;
            }
        }
    }

    DerivedSecret::new(out)
}

#[derive(Default)]
struct Writer(Zeroizing<Vec<u8>>);

impl Writer {
    fn raw(&mut self, key: Key, value: &'static str) {
        self.0.extend_from_slice(key.as_str().as_bytes());
        self.0.push(b'=');
        self.0.extend_from_slice(value.as_bytes());
        self.0.push(b'\n');
    }

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
        assert!(out.contains("wifipasswd='parola1234'\n"));
    }

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
        assert_eq!(parse_like_keyfile(&parsed[2].1), payload);
        assert!(!keys.contains(&"EVIL"));
    }

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
                    let unquoted = inner.replace("'\\''", "'");
                    assert!(
                        !unquoted.contains('\'') || inner.contains("'\\''"),
                        "unescaped quote would have ended the literal early"
                    );
                    unquoted
                } else {
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
        assert!(Ssid::parse(&"ç".repeat(16)).is_ok());
        assert!(Ssid::parse(&"ç".repeat(17)).is_err());
    }

    #[test]
    fn an_ssid_the_current_sdk_cannot_name_is_rejected() {
        for bad in ["Ağ/2", "/", "a/b/c"] {
            assert!(matches!(
                Ssid::parse(bad),
                Err(T3GemInitError::SsidUnsupportedByCurrentSdk)
            ));
        }

        for good in ["Ağ-2", "Ağ_2", r"Ağ\2", "Ağ 2"] {
            assert!(Ssid::parse(good).is_ok(), "{good} should be valid");
        }
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
    fn wifi_password_length_selects_the_interpretation() {
        let passphrase = wifi_key(&Secret::new("ThisIsAPassword")).unwrap();
        assert_eq!(*passphrase, "ThisIsAPassword");

        let psk = "0DC0D6EB90555ED6419756B9A15EC3E3209B63DF707DD508D14581F8982721AF";
        assert_eq!(*wifi_key(&Secret::new(psk)).unwrap(), psk.to_lowercase());

        for bad in ["short", &"z".repeat(64)] {
            assert!(
                matches!(
                    wifi_key(&Secret::new(bad)),
                    Err(T3GemInitError::WifiPassphraseLength)
                ),
                "{bad} should be rejected"
            );
        }

        assert_eq!(
            *wifi_key(&Secret::new("a".repeat(64))).unwrap(),
            "a".repeat(64)
        );
    }

    #[test]
    fn keyfile_escaping_covers_exactly_what_glib_decodes() {
        let cases = [
            ("parola1234", "parola1234"),
            (r"pa\ssword", r"pa\\ssword"),
            (" parola", r"\sparola"),
            ("  parola", r"\s\sparola"),
            ("parola ", "parola "),
            ("\tparola", r"\tparola"),
            (r"\ parola", r"\\ parola"),
        ];

        for (input, expected) in cases {
            assert_eq!(*escape_for_keyfile(input), expected, "escaping {input:?}");
        }
    }

    fn parse_like_keyfile(raw: &str) -> String {
        let mut chars = raw.trim_start_matches([' ', '\t']).chars();
        let mut out = String::new();

        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars
                .next()
                .expect("a trailing backslash is not a valid value")
            {
                's' => out.push(' '),
                't' => out.push('\t'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                '\\' => out.push('\\'),
                other => panic!("GLib would reject the escape \\{other}"),
            }
        }

        out
    }

    #[test]
    fn wifi_values_survive_the_shell_and_the_key_file() {
        for (ssid, password) in [
            ("Ağ-Çekirdek", "parola1234"),
            ("Ağ'ı $HOME `id`", r"pa\ssword"),
            ("  boşluklu ağ  ", "  kenarda boşluk  "),
            ("sekmeli\tağ", "sekmeli\tparola"),
            (
                "Ağ",
                "0dc0d6eb90555ed6419756b9a15ec3e3209b63df707dd508d14581f8982721af",
            ),
        ] {
            let config = T3GemInitConfig::new().with_wifi(Some(WifiSettings {
                ssid: Ssid::parse(ssid).unwrap(),
                password: Secret::new(password),
                country: WifiCountry::parse("TR").unwrap(),
            }));

            let parsed = parse_like_shell(&rendered(&config));
            let get = |k: &str| {
                parsed
                    .iter()
                    .find(|(key, _)| key == k)
                    .map(|(_, v)| v.as_str())
                    .expect("key is present")
            };

            assert_eq!(parse_like_keyfile(get("wifiname")), ssid, "ssid {ssid:?}");
            assert_eq!(
                parse_like_keyfile(get("wifipasswd")),
                password,
                "password for {ssid:?}"
            );
        }
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
