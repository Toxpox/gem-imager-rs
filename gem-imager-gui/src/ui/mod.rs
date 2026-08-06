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

    #[cfg(feature = "dfu-driver-mvp")]
    return driver_prompt::wrap(page, &state.common().dfu_driver, state.common().lang());

    #[cfg(not(feature = "dfu-driver-mvp"))]
    page
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
