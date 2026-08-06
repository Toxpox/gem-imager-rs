use gem_i18n::Msg;
use iced::{Element, widget};

use crate::{
    message::GemImagerMessage,
    state::OverlayState,
    ui::helpers::{VIEW_COL_PADDING, WINDOW_ICON, element_with_label, page_type3, selectable_text},
};

const INP_BOX_WIDTH: u32 = 420;

pub(crate) fn view<'a>(state: &'a OverlayState) -> Element<'a, GemImagerMessage> {
    let lang = state.common().lang();
    page_type3(
        review_view(state),
        [widget::button(lang.text(Msg::Back))
            .on_press(GemImagerMessage::Back)
            .style(widget::button::secondary)],
    )
}

fn review_view<'a>(state: &'a OverlayState) -> Element<'a, GemImagerMessage> {
    let lang = state.common().lang();
    let col = widget::column![
        widget::image(WINDOW_ICON.clone()),
        crate::constants::APP_NAME,
        crate::constants::APP_RELEASE,
        crate::constants::APP_DESC,
        // The full notice is in the license block below; this line is the one a user actually
        // reads, so it names both holders rather than only the fork.
        crate::constants::APP_COPYRIGHT,
        widget::rule::horizontal(2),
        element_with_label(
            lang.text(Msg::Language),
            widget::pick_list(
                gem_i18n::Lang::ALL,
                Some(lang),
                GemImagerMessage::SetLanguage,
            )
            .into()
        ),
        widget::rule::horizontal(2),
        element_with_label(
            lang.text(Msg::CacheDirectory),
            widget::text_input(&state.cache_dir, &state.cache_dir)
                .width(INP_BOX_WIDTH)
                .on_input(|_| GemImagerMessage::Null)
                .into()
        ),
        widget::rule::horizontal(2),
        element_with_label(
            lang.text(Msg::LogFile),
            widget::text_input(&state.log_path, &state.log_path)
                .width(INP_BOX_WIDTH)
                .on_input(|_| GemImagerMessage::Null)
                .into()
        ),
        widget::rule::horizontal(2),
        widget::container(selectable_text(&state.license)).padding(iced::Padding::ZERO.right(16))
    ]
    .spacing(8)
    .padding(VIEW_COL_PADDING)
    .width(iced::Fill)
    .align_x(iced::Center);

    widget::scrollable(col)
        .id(state.common().scroll_id.clone())
        .into()
}
