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

    let page = notice_modal::wrap(page, notice_for(state), state.common().lang());

    #[cfg(feature = "dfu-driver-mvp")]
    return driver_prompt::wrap(page, &state.common().dfu_driver, state.common().lang());

    #[cfg(not(feature = "dfu-driver-mvp"))]
    page
}

fn notice_for(state: &GemImager) -> Option<notice_modal::Notice> {
    match state {
        GemImager::ChooseDest(x) if x.dfu_notice => {
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
        _ => None,
    }
}

#[cfg(test)]
mod tests {
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
