use gem_i18n::{Msg, fmt};
use iced::{
    Element,
    widget::{self, text},
};

use crate::{
    constants,
    message::GemImagerMessage,
    state::CustomizeState,
    ui::helpers::{detail_pane, page_type2},
};

const HEADING_SIZE: u32 = 26;

pub(crate) fn view<'a>(state: &'a CustomizeState) -> Element<'a, GemImagerMessage> {
    let lang = state.common.lang();
    if state.erase_confirmation {
        let destination = state.selected_destination();
        let content = widget::column![
            text(lang.text(Msg::EraseConfirmTitle))
                .font(constants::FONT_BOLD)
                .size(HEADING_SIZE),
            text(fmt::erase_confirm_body(lang, &destination)),
            widget::rule::horizontal(2),
            text(destination).font(constants::FONT_BOLD),
        ]
        .spacing(16)
        .padding(24);

        return page_type2(
            content.into(),
            [
                widget::button(lang.text(Msg::EraseConfirmReject))
                    .on_press(GemImagerMessage::CancelFlashRequest)
                    .style(widget::button::secondary),
                widget::button(lang.text(Msg::EraseConfirmAccept))
                    .on_press(GemImagerMessage::FlashStart)
                    .style(widget::button::danger),
            ],
        );
    }

    let btn_label = if state.is_download() {
        lang.text(Msg::DownloadAction)
    } else {
        lang.text(Msg::WriteAction)
    };
    let action = if state.is_download() {
        GemImagerMessage::FlashStart
    } else {
        GemImagerMessage::RequestFlash
    };

    page_type2(
        review_view(state),
        [
            widget::button(lang.text(Msg::Back))
                .on_press(GemImagerMessage::Back)
                .style(widget::button::secondary),
            widget::button(btn_label).on_press(action),
        ],
    )
}

fn review_view<'a>(state: &'a CustomizeState) -> Element<'a, GemImagerMessage> {
    let lang = state.common.lang();
    let image_name = match state.selected_image.0 {
        crate::helpers::OsImageId::Format => lang.text(Msg::FormatSdCard).to_owned(),
        _ => state.selected_image.1.to_string(),
    };
    let mut col = widget::column![
        text(lang.text(Msg::ReviewTitle))
            .font(constants::FONT_BOLD)
            .size(HEADING_SIZE),
        text(lang.text(Msg::ReviewSubtitle)).style(widget::text::primary),
        widget::rule::horizontal(2),
        text(lang.text(Msg::Summary))
            .font(constants::FONT_BOLD)
            .size(HEADING_SIZE),
        widget::grid![
            text(lang.text(Msg::Device)),
            text(state.selected_board.name.as_str()),
            text(lang.text(Msg::OperatingSystem)),
            text(image_name),
            text(lang.text(Msg::Storage)),
            text(state.selected_destination())
        ]
        .height(iced::Length::Shrink)
        .spacing(8)
        .columns(2),
    ];

    // The DFU flow is the only one where the hardware has to already be in a particular state, and
    // where pulling the cable half-way through leaves the board unbootable. Those facts belong on
    // the screen that precedes the irreversible action, not in a manual.
    if state.selected_dest.is_dfu() {
        col = col.extend([
            widget::rule::horizontal(2).into(),
            text(lang.text(Msg::SpecialInstructions))
                .font(constants::FONT_BOLD)
                .size(HEADING_SIZE)
                .into(),
            dfu_instruction(
                lang.text(Msg::DfuSwitchToBootModeTitle),
                lang.text(Msg::DfuSwitchToBootModeBody),
            ),
            dfu_instruction(
                lang.text(Msg::DfuDoNotDisconnectTitle),
                lang.text(Msg::DfuDoNotDisconnectBody),
            ),
            dfu_instruction(
                lang.text(Msg::DfuSwitchBackTitle),
                lang.text(Msg::DfuSwitchBackBody),
            ),
        ]);
    }

    let modifications = state.modifications();
    if !modifications.is_empty() {
        col = col.extend([
            widget::rule::horizontal(2).into(),
            text(lang.text(Msg::ModificationsToApply))
                .font(constants::FONT_BOLD)
                .size(HEADING_SIZE)
                .into(),
            widget::column(state.modifications().into_iter().map(Into::into))
                .spacing(8)
                .into(),
        ]);
    }

    detail_pane(col, &state.common.scroll_id)
}

/// One instruction: what to do, then why it matters.
fn dfu_instruction<'a>(title: &'a str, body: &'a str) -> Element<'a, GemImagerMessage> {
    widget::column![
        text(title).font(constants::FONT_BOLD),
        text(body).style(widget::text::secondary),
    ]
    .spacing(4)
    .into()
}
