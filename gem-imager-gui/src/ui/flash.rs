use gem_i18n::Msg;
use gem_iced_widgets::progress_circle;
use iced::{
    Element,
    widget::{self, button},
};

use crate::constants::{FONT_BOLD, GEMSTONE_ROSE};
use crate::ui::helpers::{self, VIEW_COL_PADDING, detail_entry, page_type1};
use crate::{GemImagerMessage, state::FlashingState};

pub(crate) fn view(state: &FlashingState) -> Element<'_, GemImagerMessage> {
    let lang = state.common.lang();
    page_type1(
        helpers::board_view_pane(&state.selected_board, &state.common),
        progress_view(state),
        [button(lang.text(Msg::Cancel))
            .style(widget::button::danger)
            .on_press(GemImagerMessage::FlashCancel)],
    )
}

fn progress_view(state: &FlashingState) -> Element<'_, GemImagerMessage> {
    let lang = state.common.lang();
    // One monotonic axis across every pass, and a spinner exactly where there is nothing to count
    // — the board re-enumerating, or the eMMC flush. A percentage invented for those would sit
    // still for minutes and then jump, which is indistinguishable from a hang.
    let phase = state.phase();
    let indicator: Element<'_, _> = match phase.fraction {
        Some(x) => progress_circle(x, 10.0f32, GEMSTONE_ROSE, FONT_BOLD).into(),
        None => iced_aw::Spinner::new().width(80).height(80).into(),
    };

    let mut col = widget::column![indicator, widget::text(lang.text(phase.label))];
    if let Some(x) = state.time_remaining() {
        col = col.push(detail_entry(
            lang.text(Msg::TimeRemaining),
            crate::helpers::pretty_duration(x),
        ));
    }

    col.align_x(iced::Center).padding(VIEW_COL_PADDING).into()
}
