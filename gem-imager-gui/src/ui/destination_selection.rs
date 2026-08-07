use gem_i18n::Msg;
use iced::{
    Element,
    widget::{self, text},
};

use crate::{
    GemImagerMessage, constants,
    helpers::DestinationItem,
    state::ChooseDestState,
    ui::helpers::{self, detail_entry, page_type1, svg_icon_style},
};

const ICON_WIDTH: u32 = 60;

pub(crate) fn view<'a>(state: &'a ChooseDestState) -> Element<'a, GemImagerMessage> {
    let lang = state.common.lang();
    page_type1(
        dest_list_pane(state),
        dest_view_pane(state),
        [
            widget::button(lang.text(Msg::Back))
                .on_press(GemImagerMessage::Back)
                .style(widget::button::secondary),
            widget::button(lang.text(Msg::Next))
                .on_press_maybe(state.selected_dest.as_ref().map(|_| GemImagerMessage::Next)),
        ],
    )
}

fn dest_list_pane<'a>(state: &'a ChooseDestState) -> Element<'a, GemImagerMessage> {
    let items = state
        .destinations()
        .map(|dest| {
            let is_selected = state
                .selected_dest
                .as_ref()
                .map(|x| dest.is_selected(x))
                .unwrap_or(false);

            let icon: Element<GemImagerMessage> = match dest {
                DestinationItem::SaveToFile(_) => widget::svg(helpers::FILE_SAVE_ICON.clone()),
                DestinationItem::Destination(_) => widget::svg(helpers::USB_ICON.clone()),
            }
            .height(ICON_WIDTH)
            .width(ICON_WIDTH)
            .style(svg_icon_style)
            .into();

            let label: Element<'_, _> = match dest.subtitle(state.common.lang()) {
                Some(x) => widget::column![text(dest.to_string()).size(18), text(x)]
                    .width(iced::Length::Fill)
                    .into(),
                None => helpers::list_label(dest.to_string()).into(),
            };

            helpers::list_item([icon, label], is_selected, dest.msg())
        })
        .map(Into::into);

    let lang = state.common.lang();

    let filter_toggle = widget::container(
        widget::toggler(!state.filter_destination)
            .label(lang.text(Msg::ShowAllDestinations))
            .on_toggle(|x| GemImagerMessage::DestinationFilter(!x)),
    )
    .padding(16);

    let mut header: Vec<Element<'a, GemImagerMessage>> =
        vec![filter_toggle.into(), helpers::list_separator()];

    // A stand-in for the DFU target the board would expose if it were in DFU mode. It lives in the
    // header slot rather than as a `DestinationItem` variant so that it stays out of the selection
    // machinery entirely: `selected_dest` is never touched, so NEXT stays disabled on its own.
    if state.show_dfu_placeholder() {
        let icon: Element<GemImagerMessage> = widget::svg(helpers::USB_ICON.clone())
            .height(ICON_WIDTH)
            .width(ICON_WIDTH)
            .style(svg_icon_style)
            .into();

        let label: Element<GemImagerMessage> = widget::column![
            text(lang.text(Msg::DfuPlaceholderRowTitle)).size(18),
            text(lang.text(Msg::DfuDestinationSubtitle)),
        ]
        .width(iced::Length::Fill)
        .into();

        header.push(
            helpers::list_item(
                [icon, label],
                false,
                GemImagerMessage::ShowDfuNotReadyNotice,
            )
            .into(),
        );
    }

    helpers::list_pane(
        &state.search_text,
        &state.common.scroll_id,
        lang,
        header,
        items,
    )
}

fn dest_view_pane<'a>(state: &'a crate::state::ChooseDestState) -> Element<'a, GemImagerMessage> {
    match state.selected_dest.as_ref() {
        Some(dest) => {
            let icon: Element<GemImagerMessage> = widget::svg(helpers::USB_ICON.clone())
                .height(100)
                .width(iced::Fill)
                .style(svg_icon_style)
                .into();

            let col = widget::column![
                icon,
                text(dest.to_string())
                    .size(24)
                    .align_x(iced::alignment::Alignment::Center)
                    .width(iced::Length::Fill),
            ];

            let col = col.extend(
                dest.details()
                    .into_iter()
                    .map(|(k, v)| detail_entry(k, v))
                    .map(Into::into),
            );

            helpers::detail_pane(col, &state.common.scroll_id)
        }
        None => {
            let lang = state.common.lang();
            let col = widget::column![helpers::placeholder_heading(
                lang.text(Msg::SelectDestinationPrompt)
            )];

            let col = match state.instruction() {
                Some(x) => col.extend([
                    widget::rule::horizontal(2).into(),
                    text(lang.text(Msg::SpecialInstructions))
                        .size(16)
                        .font(constants::FONT_BOLD)
                        .into(),
                    text(x).into(),
                ]),
                None => col,
            };

            widget::center(helpers::detail_pane(col, &state.common.scroll_id)).into()
        }
    }
}
