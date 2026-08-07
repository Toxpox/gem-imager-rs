use iced::{
    Element,
    widget::{self, text},
};

use crate::{constants, message::GemImagerMessage};

/// Backdrop the illustration sits on.
///
/// The diagrams are black line art drawn for paper. Dropped straight onto
/// [`constants::GEMSTONE_NAVY_CARD`] they read as a blown-out white rectangle — a render fault
/// rather than a picture. A light, rounded plate around them makes the contrast look deliberate.
const MOUNT_BACKGROUND: iced::Color = iced::color!(0xf2, 0xf3, 0xf7);

/// A single-button informational modal that carries an illustration.
///
/// The handle is owned rather than borrowed: `svg::Handle` is a cheap `Arc` clone, so paying for
/// one per frame is cheaper than threading a lifetime through every caller.
pub(crate) struct Notice {
    pub(crate) illustration: widget::svg::Handle,
    pub(crate) title: gem_i18n::Msg,
    pub(crate) body: gem_i18n::Msg,
    pub(crate) dismiss_label: gem_i18n::Msg,
    pub(crate) dismiss: GemImagerMessage,
}

/// Overlays `notice` on `page`, or returns `page` untouched when there is nothing to show.
pub(crate) fn wrap<'a>(
    page: Element<'a, GemImagerMessage>,
    notice: Option<Notice>,
    lang: gem_i18n::Lang,
) -> Element<'a, GemImagerMessage> {
    let Some(notice) = notice else {
        return page;
    };

    // No `.style(..)` on the svg widget. Setting `svg::Style::color` makes resvg flood-fill the
    // rendered pixmap with one colour, collapsing every switch, digit and frame line into a single
    // indistinguishable blob. The iced 0.14 default of `{ color: None }` is what these need.
    let illustration = widget::container(
        widget::svg(notice.illustration)
            .width(iced::Fill)
            .height(iced::Shrink),
    )
    .padding(12)
    .width(iced::Fill)
    .style(|_| {
        widget::container::Style::default()
            .background(MOUNT_BACKGROUND)
            .border(iced::border::rounded(8))
    });

    let card = widget::container(
        widget::column![
            text(lang.text(notice.title))
                .font(constants::FONT_BOLD)
                .size(26),
            illustration,
            text(lang.text(notice.body)),
            widget::rule::horizontal(2),
            widget::row![
                widget::space::horizontal(),
                widget::button(lang.text(notice.dismiss_label)).on_press(notice.dismiss),
            ]
            .align_y(iced::Center),
        ]
        .spacing(18),
    )
    .padding(24)
    .width(iced::Fill)
    .max_width(constants::ILLUSTRATED_DIALOG_WIDTH)
    .style(|_| {
        widget::container::Style::default()
            .background(constants::GEMSTONE_NAVY_CARD)
            .border(iced::border::rounded(10))
    });

    let scrim = widget::opaque(
        widget::container(widget::center(card))
            .width(iced::Fill)
            .height(iced::Fill)
            .padding(24)
            .style(|_| {
                widget::container::Style::default()
                    .background(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.72))
            }),
    );

    widget::stack![page, scrim].into()
}
