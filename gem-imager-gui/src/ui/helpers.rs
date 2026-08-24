use std::sync::LazyLock;

use gem_i18n::Msg;
use gem_iced_widgets::circle_bar;
use iced::Element;
use iced::advanced::text::highlighter::PlainText;
use iced::widget::{self, svg};

use crate::{constants, message::GemImagerMessage};

pub(crate) static WINDOW_ICON: LazyLock<widget::image::Handle> =
    LazyLock::new(|| widget::image::Handle::from_bytes(constants::WINDOW_ICON_BYTES));

pub(crate) static ARROW_BACK_ICON: LazyLock<svg::Handle> =
    LazyLock::new(|| svg::Handle::from_memory(constants::ARROW_BACK_ICON_BYTES));
pub(crate) static FILE_ADD_ICON: LazyLock<svg::Handle> =
    LazyLock::new(|| svg::Handle::from_memory(constants::FILE_ADD_ICON_BYTES));
pub(crate) static USB_ICON: LazyLock<svg::Handle> =
    LazyLock::new(|| svg::Handle::from_memory(constants::USB_ICON_BYTES));
pub(crate) static FORMAT_ICON: LazyLock<svg::Handle> =
    LazyLock::new(|| svg::Handle::from_memory(constants::FORMAT_ICON_BYTES));
pub(crate) static BOARD_ICON: LazyLock<svg::Handle> =
    LazyLock::new(|| svg::Handle::from_memory(constants::BOARD_ICON_BYTES));
pub(crate) static ARROW_FORWARD_IOS_ICON: LazyLock<svg::Handle> =
    LazyLock::new(|| svg::Handle::from_memory(constants::ARROW_FORWARD_IOS_ICON_BYTES));
pub(crate) static FILE_SAVE_ICON: LazyLock<svg::Handle> =
    LazyLock::new(|| svg::Handle::from_memory(constants::FILE_SAVE_ICON_BYTES));
pub(crate) static INFO_ICON: LazyLock<svg::Handle> =
    LazyLock::new(|| svg::Handle::from_memory(constants::INFO_ICON_BYTES));
pub(crate) static COPY_ICON: LazyLock<svg::Handle> =
    LazyLock::new(|| svg::Handle::from_memory(constants::COPY_ICON_BYTES));
pub(crate) static SEARCH_ICON: LazyLock<svg::Handle> =
    LazyLock::new(|| svg::Handle::from_memory(constants::SEARCH_ICON_BYTES));

pub(crate) static USB_DFU_BOOTMODE: LazyLock<svg::Handle> =
    LazyLock::new(|| svg::Handle::from_memory(constants::USB_DFU_BOOTMODE_BYTES));
pub(crate) static EMMC_BOOTMODE: LazyLock<svg::Handle> =
    LazyLock::new(|| svg::Handle::from_memory(constants::EMMC_BOOTMODE_BYTES));

pub(crate) static BOARD_PHOTO_T3_GEM_O1: LazyLock<BoardPhoto> =
    LazyLock::new(|| BoardPhoto::new(constants::BOARD_PHOTO_T3_GEM_O1_BYTES));
pub(crate) static BOARD_PHOTO_BEAGLEY_AI: LazyLock<BoardPhoto> =
    LazyLock::new(|| BoardPhoto::new(constants::BOARD_PHOTO_BEAGLEY_AI_BYTES));

#[derive(Debug, Clone)]
pub(crate) struct BoardPhoto {
    handle: widget::image::Handle,
    aspect: f32,
}

impl BoardPhoto {
    fn new(bytes: &'static [u8]) -> Self {
        let (w, h) = image::load_from_memory(bytes)
            .map(|img| (img.width().max(1), img.height().max(1)))
            .expect("bundled board photo is not a decodable image");

        Self {
            handle: widget::image::Handle::from_bytes(bytes),
            aspect: w as f32 / h as f32,
        }
    }

    fn height_for(&self, width: f32) -> f32 {
        width / self.aspect
    }
}

pub(crate) fn board_photo(tags: &[String]) -> Option<&'static BoardPhoto> {
    tags.iter().find_map(|tag| match tag.as_str() {
        gem_config::t3::T3_BOARD_TAG => Some(&*BOARD_PHOTO_T3_GEM_O1),
        gem_config::t3::BEAGLEY_BOARD_TAG => Some(&*BOARD_PHOTO_BEAGLEY_AI),
        _ => None,
    })
}

pub(crate) fn board_list_image<'a>(
    cache: &'a gem_iced_widgets::cached_icon::Cache<url::Url>,
    tags: &[String],
    icon: Option<&url::Url>,
    width: u32,
) -> Element<'a, GemImagerMessage> {
    match board_photo(tags) {
        Some(photo) => widget::image(photo.handle.clone())
            .width(width as f32)
            .height(photo.height_for(width as f32))
            .content_fit(iced::ContentFit::Contain)
            .into(),
        None => network_image_or_default(cache, icon, BOARD_ICON.clone(), width, iced::Shrink),
    }
}

fn board_image<'a>(
    cache: &'a gem_iced_widgets::cached_icon::Cache<url::Url>,
    tags: &[String],
    icon: Option<&url::Url>,
    width: impl Into<iced::Length>,
    height: impl Into<iced::Length>,
) -> Element<'a, GemImagerMessage> {
    match board_photo(tags) {
        Some(photo) => widget::image(photo.handle.clone())
            .width(width)
            .height(height)
            .content_fit(iced::ContentFit::Contain)
            .into(),
        None => network_image_or_default(cache, icon, BOARD_ICON.clone(), width, height),
    }
}

pub(crate) const VIEW_COL_PADDING: u16 = 16;
pub(crate) const LIST_COL_PADDING: iced::Padding = iced::Padding {
    right: 16.0,
    ..iced::Padding::ZERO
};

pub(crate) fn card_btn_style(
    theme: &iced::Theme,
    status: widget::button::Status,
    is_selected: bool,
) -> widget::button::Style {
    let mut style = widget::button::Style {
        text_color: theme.palette().text,
        ..Default::default()
    };

    if is_selected || matches!(status, widget::button::Status::Hovered) {
        style.border = iced::Border::default()
            .color(theme.palette().primary)
            .width(3)
            .rounded(5);
    }

    style
}

pub(crate) fn svg_icon_style(theme: &iced::Theme, _: svg::Status) -> svg::Style {
    svg::Style {
        color: Some(theme.palette().text),
    }
}

pub(crate) fn page_type1<'a>(
    col1: Element<'a, GemImagerMessage>,
    col2: Element<'a, GemImagerMessage>,
    btns: impl IntoIterator<Item = widget::Button<'a, GemImagerMessage>>,
) -> Element<'a, GemImagerMessage> {
    let row2 = widget::row(
        [
            info_btn(INFO_ICON.clone()).into(),
            widget::space::horizontal().into(),
        ]
        .into_iter()
        .chain(btns.into_iter().map(Into::into)),
    )
    .align_y(iced::Center)
    .width(iced::Length::Fill)
    .spacing(24);

    let col2 = widget::column![
        card_box(col2)
            .height(iced::Length::Fill)
            .width(iced::Length::Fill),
        row2.width(iced::Length::Fill)
    ]
    .spacing(24)
    .width(iced::FillPortion(1));

    widget::row![
        card_box(col1)
            .height(iced::Length::Fill)
            .width(iced::Length::FillPortion(1)),
        col2
    ]
    .padding(24)
    .spacing(24)
    .into()
}

pub(crate) fn page_type2<'a>(
    row1: Element<'a, GemImagerMessage>,
    btns: impl IntoIterator<Item = widget::Button<'a, GemImagerMessage>>,
) -> Element<'a, GemImagerMessage> {
    let row2 = widget::row(
        [
            info_btn(INFO_ICON.clone()).into(),
            widget::space::horizontal().into(),
        ]
        .into_iter()
        .chain(btns.into_iter().map(Into::into)),
    )
    .align_y(iced::Center)
    .width(iced::Length::Fill)
    .spacing(24);

    widget::column![card_box(row1).height(iced::Fill).width(iced::Fill), row2]
        .padding(24)
        .spacing(24)
        .into()
}

pub(crate) fn page_type3<'a>(
    row1: Element<'a, GemImagerMessage>,
    btns: impl IntoIterator<Item = widget::Button<'a, GemImagerMessage>>,
) -> Element<'a, GemImagerMessage> {
    let row2 = widget::row(
        [widget::space::horizontal().into()]
            .into_iter()
            .chain(btns.into_iter().map(Into::into)),
    )
    .align_y(iced::Center)
    .width(iced::Length::Fill)
    .spacing(24);

    widget::column![card_box(row1).height(iced::Fill).width(iced::Fill), row2]
        .padding(24)
        .spacing(24)
        .into()
}

pub(crate) fn board_view_pane<'a>(
    dev: &'a crate::db::Board,
    state: &'a crate::GemImagerCommon,
) -> Element<'a, GemImagerMessage> {
    let img = board_image(
        &state.img_handle_cache,
        &dev.tags,
        dev.icon.as_ref(),
        iced::Fill,
        iced::Shrink,
    );

    let entry = gem_config::config::Device::from(dev);
    let copy_btn = copy_btn(COPY_ICON.clone()).on_press_with(move || {
        let json = serde_json::to_string_pretty(&entry).expect("device catalog entry serialises");
        GemImagerMessage::CopyToClipboard(json)
    });

    let cols = widget::column![
        img,
        widget::center(copy_btn),
        widget::text(&dev.name)
            .size(24)
            .align_x(iced::alignment::Alignment::Center)
            .width(iced::Length::Fill),
        widget::text(&dev.description)
            .align_x(iced::alignment::Alignment::Center)
            .width(iced::Length::Fill),
    ];

    let cols = cols.extend(
        dev.specification
            .iter()
            .map(|(k, v)| -> widget::text::Rich<'a, (), GemImagerMessage> { detail_entry(k, v) })
            .map(Into::into),
    );

    let mut btns = Vec::with_capacity(2);

    if let Some(x) = &dev.documentation {
        btns.push(
            widget::button(widget::text(state.lang().text(Msg::Documentation)))
                .on_press(GemImagerMessage::OpenUrl(x.clone()))
                .into(),
        );
    }

    if let Some(x) = &dev.oshw
        && let Ok(u) = url::Url::parse(&format!("{}/{}.html", constants::OSHW_BASE_URL, x))
    {
        btns.push(
            widget::button(widget::text("OSHW"))
                .on_press(GemImagerMessage::OpenUrl(u))
                .into(),
        );
    }

    detail_pane(
        cols.push(widget::center(widget::row(btns).spacing(16))),
        &state.scroll_id,
    )
}

pub(crate) fn detail_entry<'a>(
    key: &'a str,
    val: impl widget::text::IntoFragment<'a>,
) -> widget::text::Rich<'a, (), GemImagerMessage> {
    widget::rich_text![
        widget::span(format!("{key}:")).font(constants::FONT_BOLD),
        widget::span(" "),
        widget::span(val),
    ]
}

pub(crate) fn element_with_label<'a>(
    label: &'static str,
    el: Element<'a, GemImagerMessage>,
) -> widget::Row<'a, GemImagerMessage> {
    element_with_element(label.into(), el).padding(iced::Padding::ZERO.horizontal(16))
}

pub(crate) fn element_with_element<'a>(
    el1: Element<'a, GemImagerMessage>,
    el2: Element<'a, GemImagerMessage>,
) -> widget::Row<'a, GemImagerMessage> {
    widget::row![el1, widget::space::horizontal(), el2]
        .align_y(iced::Alignment::Center)
        .padding(iced::Padding::ZERO.right(16))
}

pub(crate) fn selectable_text(
    content: &widget::text_editor::Content,
) -> widget::text_editor::TextEditor<'_, PlainText, GemImagerMessage> {
    widget::text_editor(content).on_action(GemImagerMessage::EditorEvent)
}

fn card_box<'a>(
    content: impl Into<Element<'a, GemImagerMessage>>,
) -> widget::Container<'a, GemImagerMessage> {
    widget::container(content).style(|_| {
        widget::container::Style::default()
            .background(constants::GEMSTONE_NAVY_CARD)
            .border(iced::border::rounded(8))
    })
}

fn info_btn(handle: svg::Handle) -> widget::Button<'static, GemImagerMessage> {
    widget::button(svg(handle))
        .on_press(GemImagerMessage::AppInfo)
        .width(iced::Shrink)
        .height(iced::Shrink)
}

pub(crate) fn copy_btn<'a>(handle: svg::Handle) -> widget::Button<'a, GemImagerMessage> {
    widget::button(svg(handle))
        .width(iced::Shrink)
        .style(widget::button::secondary)
}

pub(crate) fn list_separator<'a>() -> Element<'a, GemImagerMessage> {
    widget::center(widget::rule::horizontal(2))
        .padding(iced::Padding::ZERO.left(16))
        .into()
}

pub(crate) fn list_pane<'a>(
    search_text: &'a str,
    scroll_id: &widget::Id,
    lang: gem_i18n::Lang,
    header: impl IntoIterator<Item = Element<'a, GemImagerMessage>>,
    items: impl IntoIterator<Item = Element<'a, GemImagerMessage>>,
) -> Element<'a, GemImagerMessage> {
    let top = [search_box(search_text, lang).into(), list_separator()];

    widget::scrollable(
        widget::column(top.into_iter().chain(header).chain(items)).padding(LIST_COL_PADDING),
    )
    .id(scroll_id.clone())
    .into()
}

pub(crate) fn list_item<'a>(
    contents: impl IntoIterator<Item = Element<'a, GemImagerMessage>>,
    is_selected: bool,
    msg: GemImagerMessage,
) -> widget::Button<'a, GemImagerMessage> {
    widget::button(
        widget::row(contents)
            .spacing(12)
            .padding(8)
            .align_y(iced::alignment::Vertical::Center),
    )
    .on_press(msg)
    .style(move |theme, status| card_btn_style(theme, status, is_selected))
}

pub(crate) fn list_label<'a>(label: impl widget::text::IntoFragment<'a>) -> widget::Text<'a> {
    widget::text(label).size(18).width(iced::Length::Fill)
}

pub(crate) fn detail_pane<'a>(
    content: widget::Column<'a, GemImagerMessage>,
    scroll_id: &widget::Id,
) -> Element<'a, GemImagerMessage> {
    widget::scrollable(content.spacing(16).padding(VIEW_COL_PADDING))
        .id(scroll_id.clone())
        .into()
}

pub(crate) fn placeholder_heading<'a>(label: &'a str) -> widget::Text<'a> {
    widget::text(label)
        .size(28)
        .width(iced::Fill)
        .align_x(iced::Center)
        .font(constants::FONT_BOLD)
}

pub(crate) fn placeholder_pane<'a>(label: &'a str) -> Element<'a, GemImagerMessage> {
    widget::center(placeholder_heading(label))
        .padding(VIEW_COL_PADDING)
        .into()
}

fn search_box<'a>(inp: &'a str, lang: gem_i18n::Lang) -> widget::Container<'a, GemImagerMessage> {
    widget::container(
        widget::row![
            widget::svg(SEARCH_ICON.clone())
                .style(svg_icon_style)
                .width(iced::Length::Shrink)
                .height(18),
            widget::text_input(lang.text(Msg::Search), inp)
                .style(|theme, status| {
                    let mut temp = widget::text_input::default(theme, status);
                    temp.border.width = 0.0;
                    temp.background = iced::Background::Color(iced::Color::TRANSPARENT);
                    temp
                })
                .on_input(GemImagerMessage::UpdateSearchText),
        ]
        .align_y(iced::Alignment::Center),
    )
    .padding(iced::Padding {
        left: 16.0,
        top: 16.0,
        bottom: 8.0,
        ..Default::default()
    })
}

pub(crate) fn progress_finish_view<'a>(
    label: &'static str,
    color: iced::Color,
    details: impl widget::text::IntoFragment<'a>,
) -> Element<'a, GemImagerMessage> {
    widget::column![
        circle_bar(label, 10.0f32, color, constants::FONT_BOLD),
        widget::text(details)
    ]
    .align_x(iced::Center)
    .padding(VIEW_COL_PADDING)
    .into()
}

pub(crate) fn network_image_or_default<'a>(
    cache: &'a gem_iced_widgets::cached_icon::Cache<url::Url>,
    img: Option<&url::Url>,
    def: svg::Handle,
    width: impl Into<iced::Length>,
    height: impl Into<iced::Length>,
) -> Element<'a, GemImagerMessage> {
    match img {
        Some(u) => gem_iced_widgets::cached_icon(cache, u)
            .width(width)
            .height(height)
            .into(),
        None => widget::svg(def)
            .width(width)
            .height(height)
            .style(svg_icon_style)
            .into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{BOARD_PHOTO_BEAGLEY_AI, BOARD_PHOTO_T3_GEM_O1, board_photo};

    #[test]
    fn each_board_tag_resolves_to_its_own_photo() {
        let t3 = board_photo(&[gem_config::t3::T3_BOARD_TAG.to_string()])
            .expect("T3-GEM-O1 must have a bundled photo");
        let beagley = board_photo(&[gem_config::t3::BEAGLEY_BOARD_TAG.to_string()])
            .expect("BeagleY-AI must have a bundled photo");

        assert_eq!(t3.handle, BOARD_PHOTO_T3_GEM_O1.handle);
        assert_eq!(beagley.handle, BOARD_PHOTO_BEAGLEY_AI.handle);
        assert_ne!(
            t3.handle, beagley.handle,
            "the two boards must not share one picture"
        );
    }

    #[test]
    fn an_unknown_board_has_no_bundled_photo() {
        assert!(board_photo(&[]).is_none());
        assert!(board_photo(&["beagleplay".to_string()]).is_none());
    }

    #[test]
    fn the_photo_is_found_behind_other_tags() {
        let tags = vec![
            "some-other-tag".to_string(),
            gem_config::t3::BEAGLEY_BOARD_TAG.to_string(),
        ];

        assert_eq!(
            board_photo(&tags)
                .expect("tag order must not hide the photo")
                .handle,
            BOARD_PHOTO_BEAGLEY_AI.handle
        );
    }

    #[test]
    fn board_photos_report_a_landscape_aspect_ratio() {
        for photo in [&*BOARD_PHOTO_T3_GEM_O1, &*BOARD_PHOTO_BEAGLEY_AI] {
            assert!(
                photo.aspect > 1.0,
                "board photos are landscape; got aspect {}",
                photo.aspect
            );
            assert!(photo.height_for(100.0) < 100.0);
        }
    }
}
