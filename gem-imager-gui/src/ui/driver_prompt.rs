use gem_i18n::Msg;
use iced::{
    Element,
    widget::{self, text},
};

use crate::{
    constants,
    driver_ui::{DfuDriverUiState, DriverPanel},
    message::GemImagerMessage,
};

const DIALOG_WIDTH: f32 = 680.0;

pub(crate) fn wrap<'a>(
    page: Element<'a, GemImagerMessage>,
    state: &'a DfuDriverUiState,
    lang: gem_i18n::Lang,
) -> Element<'a, GemImagerMessage> {
    let panel = state.panel();
    if matches!(panel, DriverPanel::Hidden) {
        return page;
    }

    let (title, body): (&str, Element<'a, GemImagerMessage>) = match panel {
        DriverPanel::Offer => (
            lang.text(Msg::WinusbDriverMissingTitle),
            text(lang.text(Msg::WinusbDriverMissingBody)).into(),
        ),
        DriverPanel::Details => (
            lang.text(Msg::WinusbDriverDetailsTitle),
            widget::column![
                text(lang.text(Msg::WinusbDriverDetailsBody)),
                text(lang.text(Msg::WinusbDriverCertificateNotice)).style(widget::text::secondary),
            ]
            .spacing(12)
            .into(),
        ),
        DriverPanel::Installing => (
            lang.text(Msg::WinusbDriverInstallingTitle),
            text(lang.text(Msg::WinusbDriverInstallingBody)).into(),
        ),
        DriverPanel::Ready => (
            lang.text(Msg::WinusbDriverReadyTitle),
            text(lang.text(Msg::WinusbDriverReadyBody)).into(),
        ),
        DriverPanel::Failed(error) => (
            lang.text(Msg::WinusbDriverFailedTitle),
            widget::column![
                text(lang.text(Msg::WinusbDriverFailedBody)),
                text(error).style(widget::text::secondary),
            ]
            .spacing(12)
            .into(),
        ),
        DriverPanel::Hidden => unreachable!(),
    };

    let buttons: Element<'a, GemImagerMessage> = match panel {
        DriverPanel::Offer => widget::row![
            widget::button(lang.text(Msg::WinusbDriverLater))
                .on_press(GemImagerMessage::DfuDriverDismiss)
                .style(widget::button::secondary),
            widget::button(lang.text(Msg::WinusbDriverTechnicalDetails))
                .on_press(GemImagerMessage::DfuDriverShowDetails)
                .style(widget::button::secondary),
            widget::space::horizontal(),
            widget::button(lang.text(Msg::WinusbDriverInstallAction))
                .on_press(GemImagerMessage::DfuDriverInstall),
        ]
        .align_y(iced::Center)
        .spacing(12)
        .into(),
        DriverPanel::Details => widget::row![
            widget::button(lang.text(Msg::Back))
                .on_press(GemImagerMessage::DfuDriverBackToOffer)
                .style(widget::button::secondary),
        ]
        .into(),
        DriverPanel::Installing => widget::row![].into(),
        DriverPanel::Ready => widget::row![
            widget::space::horizontal(),
            widget::button(lang.text(Msg::WinusbDriverClose))
                .on_press(GemImagerMessage::DfuDriverDismiss),
        ]
        .into(),
        DriverPanel::Failed(_) => widget::row![
            widget::button(lang.text(Msg::WinusbDriverTechnicalDetails))
                .on_press(GemImagerMessage::DfuDriverShowDetails)
                .style(widget::button::secondary),
            widget::button(lang.text(Msg::WinusbDriverClose))
                .on_press(GemImagerMessage::DfuDriverDismiss)
                .style(widget::button::secondary),
            widget::space::horizontal(),
            widget::button(lang.text(Msg::Retry)).on_press(GemImagerMessage::DfuDriverInstall),
        ]
        .align_y(iced::Center)
        .spacing(12)
        .into(),
        DriverPanel::Hidden => unreachable!(),
    };

    let card = widget::container(
        widget::column![
            text(title).font(constants::FONT_BOLD).size(26),
            body,
            widget::rule::horizontal(2),
            buttons,
        ]
        .spacing(18),
    )
    .padding(24)
    .width(iced::Fill)
    .max_width(DIALOG_WIDTH)
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
