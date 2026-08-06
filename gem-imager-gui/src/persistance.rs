//! This module contains persistance for configuration

use std::io::{Read, Write};
use std::path::PathBuf;

use gem_flasher::t3_gem_init::{self, Secret};
use serde::{Deserialize, Serialize};

/// Configuration for GUI that should be presisted
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub(crate) struct GuiConfiguration {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sd_customization: Option<SdCustomization>,

    /// The language the user picked, as an ISO 639-1 code.
    ///
    /// `None` means "never chosen", which is not the same as "English": it lets the system locale
    /// decide on every start until the user states a preference. Stored as a string rather than
    /// as [`gem_i18n::Lang`] so a config written by a future build that supports more languages
    /// still loads here — an unknown code falls back to the system locale rather than failing.
    #[serde(skip_serializing_if = "Option::is_none")]
    language: Option<String>,
}

impl GuiConfiguration {
    /// The stored language, if the user picked one this build understands.
    pub(crate) fn language(&self) -> Option<gem_i18n::Lang> {
        let stored = self.language.as_deref()?;

        match gem_i18n::Lang::from_code(stored) {
            Some(lang) => Some(lang),
            None => {
                tracing::warn!(
                    "Ignoring unsupported stored language {stored:?}; falling back to the system locale"
                );
                None
            }
        }
    }

    /// Record the user's choice. Consuming and returning `self` matches the other updaters here.
    pub(crate) fn update_language(mut self, lang: gem_i18n::Lang) -> Self {
        self.language = Some(lang.code().to_string());
        self
    }

    pub(crate) fn load() -> std::io::Result<Self> {
        let mut data = Vec::with_capacity(512);
        let config_p = Self::config_path().unwrap();

        let mut config = std::fs::File::open(config_p)?;
        config.read_to_end(&mut data)?;

        Ok(serde_json::from_slice(&data).unwrap())
    }

    pub(crate) fn save(&self) -> std::io::Result<()> {
        let data = serde_json::to_string_pretty(self).unwrap();
        let config_p = Self::config_path().unwrap();

        tracing::info!("Configuration Path: {:?}", config_p);
        std::fs::create_dir_all(config_p.parent().unwrap())?;

        let mut config = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(config_p)?;

        config.write_all(data.as_bytes())?;

        Ok(())
    }

    fn config_path() -> Option<PathBuf> {
        let dirs = crate::helpers::project_dirs()?;
        Some(dirs.config_local_dir().join("config.json").to_owned())
    }

    pub(crate) fn update_sd_customization(&mut self, t: SdCustomization) {
        self.sd_customization = Some(t);
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub(crate) struct SdCustomization {
    #[serde(skip_serializing_if = "Option::is_none")]
    sysconf: Option<SdSysconfCustomization>,
    #[serde(skip_serializing_if = "Option::is_none")]
    t3: Option<T3GemInitCustomization>,
}

impl SdCustomization {
    pub(crate) fn sysconf_customization(&self) -> Option<&SdSysconfCustomization> {
        self.sysconf.as_ref()
    }

    pub(crate) fn update_sysconfig(&mut self, t: SdSysconfCustomization) {
        self.sysconf = Some(t)
    }

    pub(crate) fn t3_customization(&self) -> Option<&T3GemInitCustomization> {
        self.t3.as_ref()
    }

    pub(crate) fn update_t3(&mut self, t: T3GemInitCustomization) {
        self.t3 = Some(t)
    }
}

/// T3 GemStone first-boot settings as the user typed them.
///
/// This is the *unvalidated* edit buffer; [`Self::build`] turns it into the validated
/// [`t3_gem_init::T3GemInitConfig`] that can actually be written.
///
/// # What is not saved
///
/// Every password field is `#[serde(skip)]`, so no secret reaches `config.json`
/// (`instruction.md` §10.3). Reloading the application therefore restores the network name but not
/// its passphrase, which is the intended trade: the alternative is a plaintext Wi-Fi key sitting in
/// the user's config directory.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub(crate) struct T3GemInitCustomization {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) hostname: Option<String>,
    #[serde(skip)]
    pub(crate) user_password: Option<Secret>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) wifi: Option<T3WifiCustomization>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) timezone: Option<chrono_tz::Tz>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) keymap: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) vnc: Option<T3VncCustomization>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub(crate) struct T3WifiCustomization {
    pub(crate) ssid: String,
    #[serde(skip)]
    pub(crate) password: Secret,
    pub(crate) country: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub(crate) struct T3VncCustomization {
    #[serde(skip)]
    pub(crate) password: Secret,
}

impl T3GemInitCustomization {
    pub(crate) fn update_hostname(mut self, t: Option<String>) -> Self {
        self.hostname = t;
        self
    }

    pub(crate) fn update_user_password(mut self, t: Option<Secret>) -> Self {
        self.user_password = t;
        self
    }

    pub(crate) fn update_wifi(mut self, t: Option<T3WifiCustomization>) -> Self {
        self.wifi = t;
        self
    }

    pub(crate) fn update_timezone(mut self, t: Option<chrono_tz::Tz>) -> Self {
        self.timezone = t;
        self
    }

    pub(crate) fn update_keymap(mut self, t: Option<String>) -> Self {
        self.keymap = t;
        self
    }

    pub(crate) fn update_vnc(mut self, t: Option<T3VncCustomization>) -> Self {
        self.vnc = t;
        self
    }

    /// Validate every field and produce the config the serializer accepts.
    ///
    /// The whole form is validated together rather than field by field, so NEXT is enabled exactly
    /// when the file can actually be written — there is no state in which the user gets past this
    /// screen and the flash then fails on a value they can no longer see.
    ///
    /// `include_vnc` comes from the selected image: the VNC fields are only offered on desktop
    /// images, and a stale VNC entry from an earlier selection must not follow the user onto a
    /// non-desktop image.
    pub(crate) fn build(
        &self,
        include_vnc: bool,
    ) -> Result<t3_gem_init::T3GemInitConfig, t3_gem_init::T3GemInitError> {
        let hostname = self
            .hostname
            .as_deref()
            .map(t3_gem_init::Hostname::parse)
            .transpose()?;

        let timezone = self
            .timezone
            .map(|tz| t3_gem_init::Timezone::parse(tz.name()))
            .transpose()?;

        let keymap = self
            .keymap
            .as_deref()
            .map(t3_gem_init::KeyboardLayout::parse)
            .transpose()?;

        let wifi = self
            .wifi
            .as_ref()
            .map(|w| {
                Ok(t3_gem_init::WifiSettings {
                    ssid: t3_gem_init::Ssid::parse(&w.ssid)?,
                    password: w.password.clone(),
                    country: t3_gem_init::WifiCountry::parse(&w.country)?,
                })
            })
            .transpose()?;

        let vnc = self
            .vnc
            .as_ref()
            .filter(|_| include_vnc)
            .map(|v| t3_gem_init::VncSettings {
                password: v.password.clone(),
            });

        let config = t3_gem_init::T3GemInitConfig::new()
            .with_hostname(hostname)
            .with_user_password(self.user_password.clone())
            .with_wifi(wifi)
            .with_timezone(timezone)
            .with_keyboard_layout(keymap)
            .with_vnc(vnc);

        // Serializing is the only way to find out whether the password lengths are acceptable, and
        // it is cheap enough to use as the validity check. The result is dropped: the bytes it
        // holds are secret, and the flash builds them again when it needs them.
        config.serialize()?;

        Ok(config)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SdSysconfCustomization {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) hostname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) timezone: Option<chrono_tz::Tz>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) keymap: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) user: Option<SdCustomizationUser>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) wifi: Option<SdCustomizationWifi>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) ssh: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) usb_enable_dhcp: Option<bool>,
}

impl Default for SdSysconfCustomization {
    fn default() -> Self {
        Self {
            hostname: None,
            timezone: None,
            keymap: None,
            user: None,
            wifi: None,
            ssh: None,
            usb_enable_dhcp: if cfg!(target_os = "macos") {
                Some(true)
            } else {
                None
            },
        }
    }
}

impl SdSysconfCustomization {
    pub(crate) fn update_hostname(mut self, t: Option<String>) -> Self {
        self.hostname = t;
        self
    }

    pub(crate) fn update_timezone(mut self, t: Option<chrono_tz::Tz>) -> Self {
        self.timezone = t;
        self
    }

    pub(crate) fn update_keymap(mut self, t: Option<String>) -> Self {
        self.keymap = t;
        self
    }

    pub(crate) fn update_user(mut self, t: Option<SdCustomizationUser>) -> Self {
        self.user = t;
        self
    }

    pub(crate) fn update_wifi(mut self, t: Option<SdCustomizationWifi>) -> Self {
        self.wifi = t;
        self
    }

    pub(crate) fn update_ssh(mut self, t: Option<String>) -> Self {
        self.ssh = t;
        self
    }

    pub(crate) fn update_usb_enable_dhcp(mut self, t: Option<bool>) -> Self {
        self.usb_enable_dhcp = t;
        self
    }

    pub(crate) fn validate_user(&self) -> bool {
        match &self.user {
            Some(x) => x.validate_username(),
            None => true,
        }
    }

    #[cfg(feature = "sd")]
    pub(crate) fn sysconfig(self) -> gem_flasher::sd::FlashingSdLinuxConfig {
        gem_flasher::sd::FlashingSdLinuxConfig::sysconfig(
            self.hostname.map(Into::into),
            self.timezone.map(|x| x.to_string()).map(Into::into),
            self.keymap.map(Into::into),
            self.user.map(|x| (x.username.into(), x.password.into())),
            self.wifi.map(|x| (x.ssid.into(), x.password.into())),
            self.ssh.map(Into::into),
            self.usb_enable_dhcp,
        )
    }

    #[cfg(feature = "sd")]
    pub(crate) fn cloudinit(self) -> gem_flasher::sd::FlashingSdLinuxConfig {
        gem_flasher::sd::FlashingSdLinuxConfig::cloud_init(
            self.hostname.map(Into::into),
            self.timezone.map(|x| x.to_string()).map(Into::into),
            self.keymap.map(Into::into),
            self.user.map(|x| (x.username.into(), x.password.into())),
            self.wifi.map(|x| (x.ssid.into(), x.password.into())),
            self.ssh.map(Into::into),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SdCustomizationUser {
    pub(crate) username: String,
    pub(crate) password: String,
}

impl SdCustomizationUser {
    pub(crate) const fn new(username: String, password: String) -> Self {
        Self { username, password }
    }

    pub(crate) fn update_username(mut self, t: String) -> Self {
        self.username = t;
        self
    }

    pub(crate) fn update_password(mut self, t: String) -> Self {
        self.password = t;
        self
    }

    pub(crate) fn validate_username(&self) -> bool {
        self.username != "root"
    }
}

impl Default for SdCustomizationUser {
    fn default() -> Self {
        Self::new(whoami::username().unwrap_or_default(), String::new())
    }
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SdCustomizationWifi {
    pub(crate) ssid: String,
    pub(crate) password: String,
}

impl SdCustomizationWifi {
    pub(crate) fn update_ssid(mut self, t: String) -> Self {
        self.ssid = t;
        self
    }

    pub(crate) fn update_password(mut self, t: String) -> Self {
        self.password = t;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sd_user_validate_rejects_root() {
        assert!(!SdCustomizationUser::new("root".into(), "pw".into()).validate_username());
        assert!(SdCustomizationUser::new("beagle".into(), "pw".into()).validate_username());
    }

    #[test]
    fn sd_user_default_has_empty_password() {
        assert!(SdCustomizationUser::default().password.is_empty());
    }

    #[test]
    fn sd_user_builders_set_fields() {
        let user = SdCustomizationUser::default()
            .update_username("alice".into())
            .update_password("secret".into());
        assert_eq!(user.username, "alice");
        assert_eq!(user.password, "secret");
    }

    #[test]
    fn sd_wifi_builders_set_fields() {
        let wifi = SdCustomizationWifi::default()
            .update_ssid("net".into())
            .update_password("pw".into());
        assert_eq!(wifi.ssid, "net");
        assert_eq!(wifi.password, "pw");
    }

    #[test]
    fn sysconf_validate_user_follows_inner_user() {
        // No user configured is always valid.
        assert!(SdSysconfCustomization::default().validate_user());
        // A configured non-root user is valid; root is not.
        let ok = SdSysconfCustomization::default()
            .update_user(Some(SdCustomizationUser::new("beagle".into(), "pw".into())));
        assert!(ok.validate_user());
        let bad = SdSysconfCustomization::default()
            .update_user(Some(SdCustomizationUser::new("root".into(), "pw".into())));
        assert!(!bad.validate_user());
    }

    #[test]
    fn sysconf_builders_populate_all_fields() {
        let cfg = SdSysconfCustomization::default()
            .update_hostname(Some("beagle".into()))
            .update_timezone(Some("UTC".parse().unwrap()))
            .update_keymap(Some("us".into()))
            .update_ssh(Some("ssh-key".into()))
            .update_usb_enable_dhcp(Some(true))
            .update_wifi(Some(
                SdCustomizationWifi::default().update_ssid("net".into()),
            ))
            .update_user(Some(SdCustomizationUser::new("beagle".into(), "pw".into())));

        assert_eq!(cfg.hostname.as_deref(), Some("beagle"));
        assert_eq!(cfg.timezone, Some(chrono_tz::Tz::UTC));
        assert_eq!(cfg.keymap.as_deref(), Some("us"));
        assert_eq!(cfg.ssh.as_deref(), Some("ssh-key"));
        assert_eq!(cfg.usb_enable_dhcp, Some(true));
        assert_eq!(cfg.wifi.as_ref().map(|w| w.ssid.as_str()), Some("net"));
        assert_eq!(
            cfg.user.as_ref().map(|u| u.username.as_str()),
            Some("beagle")
        );
    }

    #[test]
    fn sysconf_default_usb_dhcp_is_platform_specific() {
        let default = SdSysconfCustomization::default();
        if cfg!(target_os = "macos") {
            assert_eq!(default.usb_enable_dhcp, Some(true));
        } else {
            assert_eq!(default.usb_enable_dhcp, None);
        }
    }

    #[test]
    fn sd_customization_wraps_sysconf() {
        let mut sd = SdCustomization::default();
        assert!(sd.sysconf_customization().is_none());
        sd.update_sysconfig(SdSysconfCustomization::default().update_hostname(Some("bb".into())));
        assert_eq!(
            sd.sysconf_customization()
                .and_then(|s| s.hostname.as_deref()),
            Some("bb")
        );
    }

    #[test]
    fn gui_configuration_updates_each_slot() {
        let mut gui = GuiConfiguration::default();
        assert!(gui.sd_customization.is_none());

        gui.update_sd_customization(SdCustomization::default());

        assert!(gui.sd_customization.is_some());
    }

    #[test]
    fn empty_gui_configuration_serializes_to_empty_object() {
        // All fields are `skip_serializing_if = "Option::is_none"`.
        let json = serde_json::to_string(&GuiConfiguration::default()).unwrap();
        assert_eq!(json, "{}");
    }

    #[test]
    fn gui_configuration_round_trips_through_json() {
        let mut gui = GuiConfiguration::default();
        gui.update_sd_customization({
            let mut sd = SdCustomization::default();
            sd.update_sysconfig(
                SdSysconfCustomization::default().update_hostname(Some("host".into())),
            );
            sd
        });

        let json = serde_json::to_string(&gui).unwrap();
        let back: GuiConfiguration = serde_json::from_str(&json).unwrap();

        assert_eq!(
            back.sd_customization
                .and_then(|s| s.sysconf_customization().and_then(|c| c.hostname.clone())),
            Some("host".to_string())
        );
    }

    #[cfg(feature = "sd")]
    #[test]
    fn sysconf_converts_to_flasher_configs_without_panicking() {
        // Exercises the sysconfig/cloudinit bridges into gem_flasher.
        let base = SdSysconfCustomization::default()
            .update_hostname(Some("beagle".into()))
            .update_user(Some(SdCustomizationUser::new("beagle".into(), "pw".into())))
            .update_wifi(Some(
                SdCustomizationWifi::default()
                    .update_ssid("net".into())
                    .update_password("pw".into()),
            ));
        let _ = base.clone().sysconfig();
        let _ = base.cloudinit();
    }

    fn t3_form() -> T3GemInitCustomization {
        T3GemInitCustomization::default()
            .update_hostname(Some("t3-gemstone".into()))
            .update_user_password(Some(Secret::new("account-secret")))
            .update_wifi(Some(T3WifiCustomization {
                ssid: "Ağ-Çekirdek".into(),
                password: Secret::new("wifi-secret"),
                country: "tr".into(),
            }))
            .update_timezone(Some(chrono_tz::Europe::Istanbul))
            .update_keymap(Some("tr".into()))
            .update_vnc(Some(T3VncCustomization {
                password: Secret::new("vnc1234"),
            }))
    }

    /// `instruction.md` §10.3: no secret may be persisted. This checks the serialized JSON text
    /// directly, because that is the artefact that ends up on disk.
    #[test]
    fn no_t3_secret_is_ever_written_to_the_config_file() {
        let mut gui = GuiConfiguration::default();
        gui.update_sd_customization({
            let mut sd = SdCustomization::default();
            sd.update_t3(t3_form());
            sd
        });

        let json = serde_json::to_string(&gui).unwrap();

        for secret in ["account-secret", "wifi-secret", "vnc1234"] {
            assert!(!json.contains(secret), "{secret} was persisted");
        }
        // The non-secret settings are still saved, so restoring is worth doing at all.
        assert!(json.contains("t3-gemstone"));
        assert!(json.contains("Ağ-Çekirdek"));
    }

    /// Reloading restores the network name but leaves its passphrase blank, rather than restoring a
    /// wrong or stale one.
    #[test]
    fn reloading_restores_settings_but_not_passwords() {
        let mut gui = GuiConfiguration::default();
        gui.update_sd_customization({
            let mut sd = SdCustomization::default();
            sd.update_t3(t3_form());
            sd
        });

        let json = serde_json::to_string(&gui).unwrap();
        let back: GuiConfiguration = serde_json::from_str(&json).unwrap();
        let t3 = back
            .sd_customization
            .and_then(|s| s.t3_customization().cloned())
            .expect("the T3 section survives a round trip");

        assert_eq!(t3.hostname.as_deref(), Some("t3-gemstone"));
        assert_eq!(t3.timezone, Some(chrono_tz::Europe::Istanbul));
        assert_eq!(
            t3.wifi.as_ref().map(|w| w.ssid.as_str()),
            Some("Ağ-Çekirdek")
        );
        assert!(t3.wifi.unwrap().password.is_empty());
        assert!(t3.user_password.is_none());
    }

    #[test]
    fn a_complete_form_builds_and_a_bad_hostname_does_not() {
        assert!(t3_form().build(true).is_ok());

        let err = t3_form()
            .update_hostname(Some("-not a hostname".into()))
            .build(true)
            .unwrap_err();
        assert!(matches!(err, t3_gem_init::T3GemInitError::InvalidHostname));
    }

    /// The VNC fields only exist on desktop images, so a VNC entry left over from an earlier
    /// selection must not be written when a non-desktop image is chosen.
    #[test]
    fn vnc_is_dropped_for_non_desktop_images() {
        let desktop = t3_form().build(true).unwrap();
        assert!(desktop.vnc_secret_survives_first_boot());

        let console = t3_form().build(false).unwrap();
        assert!(!console.vnc_secret_survives_first_boot());
        let rendered = String::from_utf8(console.serialize().unwrap().to_vec()).unwrap();
        assert!(!rendered.contains("vnc"));
    }

    /// A password over the protocol limit fails the whole form, so NEXT stays disabled instead of
    /// the flash failing later.
    #[test]
    fn an_over_long_vnc_password_fails_validation() {
        let err = t3_form()
            .update_vnc(Some(T3VncCustomization {
                password: Secret::new("123456789"),
            }))
            .build(true)
            .unwrap_err();

        assert!(matches!(
            err,
            t3_gem_init::T3GemInitError::VncPasswordTooLong { len: 9 }
        ));
    }

    /// The keymap picker and the serializer's allowlist have to agree, or the user could select a
    /// layout the file refuses to carry.
    #[test]
    fn every_offered_keymap_is_accepted_by_the_serializer() {
        for keymap in crate::constants::KEYMAP_LAYOUTS {
            assert!(
                t3_gem_init::KeyboardLayout::parse(keymap).is_ok(),
                "{keymap} is offered by the picker but rejected by the serializer"
            );
        }
        assert_eq!(
            crate::constants::KEYMAP_LAYOUTS,
            t3_gem_init::KEYBOARD_LAYOUTS
        );
    }
}
