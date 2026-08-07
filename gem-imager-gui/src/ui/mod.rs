use gem_i18n::Msg;

use crate::{GemImager, message::GemImagerMessage};

mod app_info;
mod board_selection;
mod configuration;
mod destination_selection;
#[cfg(feature = "dfu-driver-mvp")]
mod driver_prompt;
mod flash;
mod flash_finish;
mod helpers;
mod image_selection;
mod notice_modal;
mod review;

pub(crate) fn view(state: &GemImager) -> iced::Element<'_, GemImagerMessage> {
    let page = match state {
        GemImager::ChooseBoard(inner) => board_selection::view(inner),
        GemImager::ChooseOs(inner) => image_selection::view(inner),
        GemImager::ChooseDest(inner) => destination_selection::view(inner),
        GemImager::Customize(inner) => configuration::view(inner),
        GemImager::Review(inner) => review::view(inner),
        GemImager::Flashing(inner) => flash::view(inner),
        GemImager::FlashingCancel(inner) => flash_finish::cancel(inner),
        GemImager::FlashingFail(inner) => flash_finish::fail(inner),
        GemImager::FlashingSuccess(inner) => flash_finish::success(inner),
        GemImager::AppInfo(inner) => app_info::view(inner),
        _ => panic!("Unexpected message"),
    };

    // Inner layer. A missing WinUSB driver is the more actionable of the two problems, so
    // `driver_prompt` goes on top of this one rather than under it.
    let page = notice_modal::wrap(page, notice_for(state), state.common().lang());

    #[cfg(feature = "dfu-driver-mvp")]
    return driver_prompt::wrap(page, &state.common().dfu_driver, state.common().lang());

    #[cfg(not(feature = "dfu-driver-mvp"))]
    page
}

/// The illustrated notice the current screen wants, if any.
///
/// Deliberately keyed on the `GemImager` variant rather than on state fields alone:
/// `FlashingCancel` and `FlashingSuccess` share one `FlashingFinishState`, so a field-only test
/// would fire the "you are done, switch back to eMMC" notice on a cancelled write too.
fn notice_for(state: &GemImager) -> Option<notice_modal::Notice> {
    match state {
        GemImager::ChooseDest(x) if x.dfu_notice => {
            // A board that is in DFU mode but has no driver bound enumerates as nothing, which
            // looks identical to no board at all. Telling that user to move the switches is wrong,
            // and `driver_prompt` is already on screen with the right answer.
            #[cfg(feature = "dfu-driver-mvp")]
            if state.common().dfu_driver.device_present() {
                return None;
            }

            Some(notice_modal::Notice {
                illustration: helpers::USB_DFU_BOOTMODE.clone(),
                title: Msg::DfuNotConnectedTitle,
                body: Msg::DfuNotConnectedBody,
                dismiss_label: Msg::WinusbDriverClose,
                dismiss: GemImagerMessage::DismissNotice,
            })
        }
        GemImager::FlashingSuccess(x) if x.is_dfu && !x.notice_dismissed => {
            Some(notice_modal::Notice {
                illustration: helpers::EMMC_BOOTMODE.clone(),
                title: Msg::DfuSwitchBackTitle,
                body: Msg::DfuSwitchBackBody,
                dismiss_label: Msg::WinusbDriverClose,
                dismiss: GemImagerMessage::DismissNotice,
            })
        }
        // Everything else, `AppInfo` included. An `AppInfo` overlay opened from the success screen
        // still carries `OverlayData::FlashingSuccess`, and a notice rendered over it would leave
        // an undismissable scrim on a screen that has no dismiss button of its own.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    /// Review guard for `instruction.md` §11.2.
    ///
    /// Data supplied by catalogs/devices and technical placeholders may remain dynamic, but
    /// buttons, toggler labels, headings and detail labels owned by this application must go
    /// through `gem-i18n`. This intentionally checks source text: a new English literal should
    /// fail in the same change that introduces it, before anybody has to notice it visually.
    #[test]
    fn user_facing_ui_controls_do_not_embed_runtime_literals() {
        let files = [
            ("app_info", include_str!("app_info.rs")),
            ("board_selection", include_str!("board_selection.rs")),
            ("configuration", include_str!("configuration.rs")),
            (
                "destination_selection",
                include_str!("destination_selection.rs"),
            ),
            #[cfg(feature = "dfu-driver-mvp")]
            ("driver_prompt", include_str!("driver_prompt.rs")),
            ("flash", include_str!("flash.rs")),
            ("flash_finish", include_str!("flash_finish.rs")),
            ("image_selection", include_str!("image_selection.rs")),
            ("notice_modal", include_str!("notice_modal.rs")),
            ("review", include_str!("review.rs")),
        ];
        let forbidden = [
            "widget::button(\"",
            "button(\"",
            ".label(\"",
            "placeholder_pane(\"",
            "placeholder_heading(\"",
            "detail_entry(\"",
            "text(\"",
        ];

        for (name, source) in files {
            for pattern in forbidden {
                assert!(
                    !source.contains(pattern),
                    "{name}.rs contains user-facing literal pattern {pattern:?}; add a gem-i18n Msg key"
                );
            }
        }
    }
}
