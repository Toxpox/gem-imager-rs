use iced::{
    Element,
    widget::{self, text},
};

use bb_flasher::t3_gem_init::{Hostname, Secret, Ssid, WifiCountry};
use bb_i18n::Msg;

use crate::{
    BBImagerMessage,
    helpers::{self, FlashingCustomization},
    persistance,
    ui::helpers::{detail_pane, element_with_element, element_with_label, page_type2},
};

const INPUT_WIDTH: u32 = 200;

pub(crate) fn view<'a>(state: &'a crate::state::CustomizeState) -> Element<'a, BBImagerMessage> {
    let lang = state.common.lang();
    page_type2(
        customization_pane(state),
        [
            widget::button(lang.text(Msg::Reset))
                .style(widget::button::danger)
                .on_press(BBImagerMessage::ResetFlashingConfig),
            widget::button(lang.text(Msg::Back))
                .on_press(BBImagerMessage::Back)
                .style(widget::button::secondary),
            widget::button(lang.text(Msg::Next)).on_press_maybe(
                if state.customization.validate() {
                    Some(BBImagerMessage::Next)
                } else {
                    None
                },
            ),
        ],
    )
}

fn customization_pane<'a>(state: &'a crate::state::CustomizeState) -> Element<'a, BBImagerMessage> {
    match &state.customization {
        FlashingCustomization::LinuxSdSysconfig(inner) => linux_sd_card_sysconfig(state, inner),
        FlashingCustomization::LinuxSdCloudInit(inner) => linux_sd_card_cloudinit(state, inner),
        FlashingCustomization::T3GemInit { config, desktop } => {
            t3_gem_init(state, config, *desktop)
        }
        _ => panic!("No customization"),
    }
}

/// The T3 GemStone first-boot screen.
///
/// Only the fields the current `gem-first-boot` consumer actually reads are here
/// (`instruction.md` §10.1). Disk encryption, SD-to-eMMC copy, the USB gadget toggles and the SSH
/// options are deliberately absent: the consumer ignores those keys, so a switch for them would
/// claim to configure something it does not.
fn t3_gem_init<'a>(
    state: &'a crate::state::CustomizeState,
    config: &'a persistance::T3GemInitCustomization,
    desktop: bool,
) -> Element<'a, BBImagerMessage> {
    let lang = state.common.lang();
    let wrap = move |c: persistance::T3GemInitCustomization| FlashingCustomization::T3GemInit {
        config: c,
        desktop,
    };

    let mut col = widget::column([]);

    // Account password
    col = col.push(
        widget::toggler(config.user_password.is_some())
            .label(lang.text(Msg::SetPassword))
            .on_toggle(move |t| {
                let c = if t { Some(Default::default()) } else { None };
                BBImagerMessage::UpdateFlashConfig(wrap(config.clone().update_user_password(c)))
            }),
    );
    if let Some(password) = config.user_password.as_ref() {
        col = col.push(secret_input_with_label(
            lang.text(Msg::Password),
            "password",
            password,
            move |inp| wrap(config.clone().update_user_password(Some(inp.into()))),
        ));
    }

    col = col.push(widget::rule::horizontal(2));

    // Wireless network
    col = col.push(
        widget::toggler(config.wifi.is_some())
            .label(lang.text(Msg::ConfigureWirelessLan))
            .on_toggle(move |t| {
                let c = if t { Some(Default::default()) } else { None };
                BBImagerMessage::UpdateFlashConfig(wrap(config.clone().update_wifi(c)))
            }),
    );
    if let Some(wifi) = config.wifi.as_ref() {
        col = col.extend([
            input_with_label(
                lang.text(Msg::Ssid),
                "SSID",
                &wifi.ssid,
                move |inp| {
                    let mut w = wifi.clone();
                    w.ssid = inp;
                    wrap(config.clone().update_wifi(Some(w)))
                },
                Ssid::parse(&wifi.ssid).is_err(),
            )
            .into(),
            secret_input_with_label(
                lang.text(Msg::Password),
                "passphrase or 64-digit key",
                &wifi.password,
                move |inp| {
                    let mut w = wifi.clone();
                    w.password = inp.into();
                    wrap(config.clone().update_wifi(Some(w)))
                },
            )
            .into(),
            input_with_label(
                lang.text(Msg::Country),
                "TR",
                &wifi.country,
                move |inp| {
                    let mut w = wifi.clone();
                    w.country = inp;
                    wrap(config.clone().update_wifi(Some(w)))
                },
                WifiCountry::parse(&wifi.country).is_err(),
            )
            .into(),
            hint(lang.text(Msg::WifiKeyHint)),
        ]);
    }

    col = col.push(widget::rule::horizontal(2));

    // Timezone
    let toggle = widget::toggler(config.timezone.is_some())
        .label(lang.text(Msg::SetTimezone))
        .on_toggle(move |t| {
            let tz = if t { helpers::system_timezone() } else { None };
            BBImagerMessage::UpdateFlashConfig(wrap(config.clone().update_timezone(tz)))
        });
    col = match config.timezone.as_ref() {
        Some(tz) => {
            // The combo box callback outlives this borrow, so it gets its own copy of the buffer.
            let xc = config.clone();
            col.push(element_with_element(
                toggle.into(),
                widget::combo_box(
                    &state.common.timezones,
                    lang.text(Msg::Timezone),
                    Some(tz),
                    move |t| {
                        BBImagerMessage::UpdateFlashConfig(wrap(
                            xc.clone().update_timezone(Some(t)),
                        ))
                    },
                )
                .width(INPUT_WIDTH)
                .into(),
            ))
        }
        None => col.push(toggle),
    };

    col = col.push(widget::rule::horizontal(2));

    // Hostname
    let toggle = widget::toggler(config.hostname.is_some())
        .label(lang.text(Msg::SetHostname))
        .on_toggle(move |t| {
            let hostname = if t { Some(String::new()) } else { None };
            BBImagerMessage::UpdateFlashConfig(wrap(config.clone().update_hostname(hostname)))
        });
    col = match config.hostname.as_ref() {
        Some(hostname) => col.push(element_with_element(
            toggle.into(),
            widget::text_input("gemstone", hostname)
                .on_input(move |inp| {
                    BBImagerMessage::UpdateFlashConfig(wrap(
                        config.clone().update_hostname(Some(inp)),
                    ))
                })
                .style(move |theme, status| {
                    invalid_style(theme, status, Hostname::parse(hostname).is_err())
                })
                .width(INPUT_WIDTH)
                .into(),
        )),
        None => col.push(toggle),
    };

    col = col.push(widget::rule::horizontal(2));

    // Keymap
    let toggle = widget::toggler(config.keymap.is_some())
        .label(lang.text(Msg::SetKeymap))
        .on_toggle(move |t| {
            let keymap = if t {
                Some(helpers::system_keymap().to_string())
            } else {
                None
            };
            BBImagerMessage::UpdateFlashConfig(wrap(config.clone().update_keymap(keymap)))
        });
    col = match config.keymap.as_ref() {
        Some(keymap) => {
            let xc = config.clone();
            let options = state.common.keymaps.options();
            let selection = options
                .binary_search(&keymap.as_str())
                .map(|x| &options[x])
                .ok();

            col.push(element_with_element(
                toggle.into(),
                widget::combo_box(
                    &state.common.keymaps,
                    lang.text(Msg::Keymap),
                    selection,
                    move |t| {
                        BBImagerMessage::UpdateFlashConfig(wrap(
                            xc.clone().update_keymap(Some(t.to_string())),
                        ))
                    },
                )
                .width(INPUT_WIDTH)
                .into(),
            ))
        }
        None => col.push(toggle),
    };

    // VNC — desktop images only.
    if desktop {
        col = col.push(widget::rule::horizontal(2));
        col = col.push(
            widget::toggler(config.vnc.is_some())
                .label(lang.text(Msg::EnableVnc))
                .on_toggle(move |t| {
                    let c = if t { Some(Default::default()) } else { None };
                    BBImagerMessage::UpdateFlashConfig(wrap(config.clone().update_vnc(c)))
                }),
        );
        if let Some(vnc) = config.vnc.as_ref() {
            col = col.extend([
                secret_input_with_label(
                    lang.text(Msg::Password),
                    "up to 8 characters",
                    &vnc.password,
                    move |inp| {
                        wrap(
                            config
                                .clone()
                                .update_vnc(Some(persistance::T3VncCustomization {
                                    password: inp.into(),
                                })),
                        )
                    },
                )
                .into(),
                // Both facts are stated rather than hidden: the length limit is the protocol's, not
                // the application's, and the leftover secret is a known SDK defect
                // (`instruction.md` §10.5).
                hint(lang.text(Msg::VncProtocolHint)),
                hint(lang.text(Msg::VncKnownIssueHint)),
            ]);
        }
    }

    // Whatever is wrong with the form right now, shown next to the disabled NEXT button.
    if let Some(err) = state.customization.validation_error(lang) {
        col = col.extend([
            widget::rule::horizontal(2).into(),
            error_text(err.to_owned()),
        ]);
    }

    detail_pane(col, &state.common.scroll_id)
}

/// A password field: masked on screen, and never rendered from a plain `String`.
fn secret_input_with_label<'a, F>(
    label: &'static str,
    placeholder: &'static str,
    value: &'a Secret,
    update_config_cb: F,
) -> widget::Row<'a, BBImagerMessage>
where
    F: 'a + Fn(String) -> FlashingCustomization,
{
    element_with_label(
        label,
        widget::text_input(placeholder, value.as_input())
            .secure(true)
            .on_input(move |inp| BBImagerMessage::UpdateFlashConfig(update_config_cb(inp)))
            .width(INPUT_WIDTH)
            .into(),
    )
}

/// Explanatory text under a field. Wrapped rather than truncated: every hint here is a fact the
/// user needs before flashing, not decoration.
fn hint(message: &str) -> Element<'_, BBImagerMessage> {
    widget::container(text(message.to_owned()).size(12))
        .padding(iced::Padding::ZERO.horizontal(16))
        .into()
}

fn error_text(message: String) -> Element<'static, BBImagerMessage> {
    widget::container(
        text(message)
            .size(12)
            .style(|theme: &iced::Theme| widget::text::Style {
                color: Some(theme.palette().danger),
            }),
    )
    .padding(iced::Padding::ZERO.horizontal(16))
    .into()
}

fn invalid_style(
    theme: &iced::Theme,
    status: widget::text_input::Status,
    invalid: bool,
) -> widget::text_input::Style {
    let mut t = widget::text_input::default(theme, status);
    if invalid {
        t.border = t.border.color(theme.palette().danger);
    }
    t
}

fn linux_sd_card_common<'a>(
    state: &'a crate::state::CustomizeState,
    config: &'a persistance::SdSysconfCustomization,
    wrap: impl Fn(persistance::SdSysconfCustomization) -> FlashingCustomization + Copy + 'static,
) -> widget::Column<'a, BBImagerMessage> {
    let lang = state.common.lang();
    let mut col = widget::column([]);

    // Username and Password
    col = col.push(
        widget::toggler(config.user.is_some())
            .label(lang.text(Msg::ConfigureUsernamePassword))
            .on_toggle(move |t| {
                let c = if t { Some(Default::default()) } else { None };
                BBImagerMessage::UpdateFlashConfig(wrap(config.clone().update_user(c)))
            }),
    );
    if let Some(usr) = config.user.as_ref() {
        col = col.extend([
            input_with_label(
                lang.text(Msg::Username),
                "username",
                &usr.username,
                move |inp| {
                    wrap(
                        config
                            .clone()
                            .update_user(Some(usr.clone().update_username(inp))),
                    )
                },
                !usr.validate_username(),
            )
            .into(),
            input_with_label(
                lang.text(Msg::Password),
                "password",
                &usr.password,
                move |inp| {
                    wrap(
                        config
                            .clone()
                            .update_user(Some(usr.clone().update_password(inp))),
                    )
                },
                false,
            )
            .into(),
        ])
    }

    col = col.push(widget::rule::horizontal(2));

    // Wifi
    col = col.push(
        widget::toggler(config.wifi.is_some())
            .label(lang.text(Msg::ConfigureWirelessLan))
            .on_toggle(move |t| {
                let c = if t { Some(Default::default()) } else { None };
                BBImagerMessage::UpdateFlashConfig(wrap(config.clone().update_wifi(c)))
            }),
    );
    if let Some(wifi) = config.wifi.as_ref() {
        col = col.extend([
            input_with_label(
                lang.text(Msg::Ssid),
                "SSID",
                &wifi.ssid,
                move |inp| {
                    wrap(
                        config
                            .clone()
                            .update_wifi(Some(wifi.clone().update_ssid(inp))),
                    )
                },
                false,
            )
            .into(),
            input_with_label(
                lang.text(Msg::Password),
                "password",
                &wifi.password,
                move |inp| {
                    wrap(
                        config
                            .clone()
                            .update_wifi(Some(wifi.clone().update_password(inp))),
                    )
                },
                false,
            )
            .into(),
        ])
    };

    col = col.push(widget::rule::horizontal(2));

    // Timezone
    let toggle = widget::toggler(config.timezone.is_some())
        .label(lang.text(Msg::SetTimezone))
        .on_toggle(move |t| {
            let tz = if t { helpers::system_timezone() } else { None };
            BBImagerMessage::UpdateFlashConfig(wrap(config.clone().update_timezone(tz)))
        });
    col = match config.timezone.as_ref() {
        Some(tz) => {
            let xc = config.clone();
            // The configuration stores the zone as a string, so it has to be resolved
            // back to a `Tz` for the combo box to show it as the current selection.
            col.push(element_with_element(
                toggle.into(),
                widget::combo_box(
                    &state.common.timezones,
                    lang.text(Msg::Timezone),
                    Some(tz),
                    move |t| {
                        BBImagerMessage::UpdateFlashConfig(wrap(
                            xc.clone().update_timezone(Some(t)),
                        ))
                    },
                )
                .width(INPUT_WIDTH)
                .into(),
            ))
        }
        None => col.push(toggle),
    };

    col = col.push(widget::rule::horizontal(2));

    // Hostname
    let toggle = widget::toggler(config.hostname.is_some())
        .label(lang.text(Msg::SetHostname))
        .on_toggle(move |t| {
            let hostname = if t { Some(String::new()) } else { None };
            BBImagerMessage::UpdateFlashConfig(wrap(config.clone().update_hostname(hostname)))
        });
    col = match config.hostname.as_ref() {
        Some(hostname) => col.push(element_with_element(
            toggle.into(),
            widget::text_input("beagle", hostname)
                .on_input(move |inp| {
                    BBImagerMessage::UpdateFlashConfig(wrap(
                        config.clone().update_hostname(Some(inp)),
                    ))
                })
                .width(INPUT_WIDTH)
                .into(),
        )),
        None => col.push(toggle),
    };

    col = col.push(widget::rule::horizontal(2));

    // Keymap
    let toggle = widget::toggler(config.keymap.is_some())
        .label(lang.text(Msg::SetKeymap))
        .on_toggle(move |t| {
            let keymap = if t {
                Some(helpers::system_keymap().to_string())
            } else {
                None
            };
            BBImagerMessage::UpdateFlashConfig(wrap(config.clone().update_keymap(keymap)))
        });
    col = match config.keymap.as_ref() {
        Some(keymap) => {
            let xc = config.clone();
            // The current selection needs to be resolved back to one of the options,
            // which are kept sorted (see `constants::KEYMAP_LAYOUTS`) to allow a binary
            // search.
            let options = state.common.keymaps.options();
            let selection = options
                .binary_search(&keymap.as_str())
                .map(|x| &options[x])
                .ok();

            col.push(element_with_element(
                toggle.into(),
                widget::combo_box(
                    &state.common.keymaps,
                    lang.text(Msg::Keymap),
                    selection,
                    move |t| {
                        BBImagerMessage::UpdateFlashConfig(wrap(
                            xc.clone().update_keymap(Some(t.to_string())),
                        ))
                    },
                )
                .width(INPUT_WIDTH)
                .into(),
            ))
        }
        None => col.push(toggle),
    };

    col = col.push(widget::rule::horizontal(2));

    // SSH Key
    col.extend([
        text(lang.text(Msg::SshAuthorizationKey)).into(),
        widget::center(
            widget::text_input("authorized key", config.ssh.as_deref().unwrap_or("")).on_input(
                move |x| {
                    BBImagerMessage::UpdateFlashConfig(wrap(
                        config
                            .clone()
                            .update_ssh(if x.is_empty() { None } else { Some(x) }),
                    ))
                },
            ),
        )
        .padding(iced::Padding::ZERO.horizontal(16))
        .into(),
    ])
}

fn linux_sd_card_cloudinit<'a>(
    state: &'a crate::state::CustomizeState,
    config: &'a persistance::SdSysconfCustomization,
) -> Element<'a, BBImagerMessage> {
    let col = linux_sd_card_common(state, config, FlashingCustomization::LinuxSdCloudInit);
    detail_pane(col, &state.common.scroll_id)
}

fn linux_sd_card_sysconfig<'a>(
    state: &'a crate::state::CustomizeState,
    config: &'a persistance::SdSysconfCustomization,
) -> Element<'a, BBImagerMessage> {
    let mut col = linux_sd_card_common(state, config, FlashingCustomization::LinuxSdSysconfig);

    col = col.push(widget::rule::horizontal(2));
    // Enable USB DHCP
    col = col.push(
        widget::toggler(config.usb_enable_dhcp == Some(true))
            .label(state.common.lang().text(Msg::EnableUsbDhcp))
            .on_toggle(|x| {
                BBImagerMessage::UpdateFlashConfig(FlashingCustomization::LinuxSdSysconfig(
                    config.clone().update_usb_enable_dhcp(Some(x)),
                ))
            }),
    );

    detail_pane(col, &state.common.scroll_id)
}

fn input_with_label<'a, F>(
    label: &'static str,
    placeholder: &'static str,
    val: &'a str,
    update_config_cb: F,
    invalid_val: bool,
) -> widget::Row<'a, BBImagerMessage>
where
    F: 'a + Fn(String) -> FlashingCustomization,
{
    element_with_label(
        label,
        widget::text_input(placeholder, val)
            .on_input(move |inp| BBImagerMessage::UpdateFlashConfig(update_config_cb(inp)))
            .style(move |theme, status| {
                let mut t = widget::text_input::default(theme, status);

                if invalid_val {
                    t.border = t.border.color(theme.palette().danger);
                    t
                } else {
                    t
                }
            })
            .width(INPUT_WIDTH)
            .into(),
    )
}
