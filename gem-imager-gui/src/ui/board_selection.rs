use gem_i18n::Msg;
use iced::{Element, widget};

use crate::{GemImagerMessage, state::ChooseBoardState, ui::helpers};

const ICON_WIDTH: u32 = 100;

pub(crate) fn view<'a>(state: &'a ChooseBoardState) -> Element<'a, GemImagerMessage> {
    let lang = state.common.lang();
    helpers::page_type1(
        board_list_pane(state),
        board_view_pane(state),
        [widget::button(lang.text(Msg::Next)).on_press_maybe(
            state
                .selected_board
                .as_ref()
                .map(|_| GemImagerMessage::Next),
        )],
    )
}

fn board_list_pane<'a>(state: &'a ChooseBoardState) -> Element<'a, GemImagerMessage> {
    let items = state
        .boards
        .iter()
        .map(|dev| {
            let is_selected = state
                .selected_board
                .as_ref()
                .map(|x| x.id == dev.id)
                .unwrap_or(false);
            let img = helpers::network_image_or_default(
                &state.common.img_handle_cache,
                dev.icon.as_ref(),
                helpers::BOARD_ICON.clone(),
                ICON_WIDTH,
                iced::Shrink,
            );
            helpers::list_item(
                [img, helpers::list_label(&dev.name).into()],
                is_selected,
                GemImagerMessage::SelectBoardById(dev.id),
            )
        })
        .map(Into::into);

    helpers::list_pane(
        &state.search_text,
        &state.common.scroll_id,
        state.common.lang(),
        [],
        items,
    )
}

fn board_view_pane<'a>(state: &'a ChooseBoardState) -> Element<'a, GemImagerMessage> {
    match state.selected_board.as_ref() {
        Some(dev) => helpers::board_view_pane(dev, &state.common),
        None => helpers::placeholder_pane(state.common.lang().text(Msg::SelectBoardPrompt)),
    }
}
