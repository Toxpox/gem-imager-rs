use gem_i18n::Msg;
use iced::Element;
use iced::widget::{self, button};

use crate::state::{FlashingFailState, FlashingFinishState};
use crate::ui::helpers::{board_view_pane, page_type1, progress_finish_view, selectable_text};
use crate::{GemImagerMessage, constants};

pub(crate) fn fail(state: &FlashingFailState) -> Element<'_, GemImagerMessage> {
    let lang = state.common.lang();
    page_type1(
        info_view(state),
        progress_finish_view(lang.text(Msg::Failed), constants::DANGER_RED, &state.err),
        [
            button(lang.text(Msg::FlashNew))
                .style(widget::button::danger)
                .on_press(GemImagerMessage::Restart),
            button(lang.text(Msg::Retry))
                .style(widget::button::primary)
                .on_press(GemImagerMessage::Retry),
        ],
    )
}

pub(crate) fn info_view(state: &FlashingFailState) -> Element<'_, GemImagerMessage> {
    widget::column![
        widget::text(state.common.lang().text(Msg::Logs))
            .size(28)
            .font(constants::FONT_BOLD),
        widget::rule::horizontal(2),
        selectable_text(&state.logs)
    ]
    .spacing(8)
    .padding(crate::ui::helpers::VIEW_COL_PADDING)
    .into()
}

pub(crate) fn cancel(state: &FlashingFinishState) -> Element<'_, GemImagerMessage> {
    let lang = state.common.lang();
    page_type1(
        board_view_pane(&state.selected_board, &state.common),
        progress_finish_view(
            lang.text(Msg::Cancelled),
            constants::DANGER_RED,
            lang.text(Msg::CancelledByUser),
        ),
        [button(lang.text(Msg::Restart))
            .style(widget::button::danger)
            .on_press(GemImagerMessage::Restart)],
    )
}

pub(crate) fn success(state: &FlashingFinishState) -> Element<'_, GemImagerMessage> {
    let lang = state.common.lang();
    let msg = if state.is_download {
        lang.text(Msg::DownloadSuccess)
    } else {
        lang.text(Msg::FlashSuccess)
    };

    page_type1(
        board_view_pane(&state.selected_board, &state.common),
        progress_finish_view("100%", constants::SUCCESS_GREEN, msg),
        [button(lang.text(Msg::FlashAnother))
            .style(widget::button::primary)
            .on_press(GemImagerMessage::Restart)],
    )
}
