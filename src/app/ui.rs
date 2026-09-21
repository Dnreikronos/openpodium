//! Reusable view helpers built on top of the [`shell`] design tokens.
//!
//! These wrap the raw Iced widgets so that every panel gets the same controls,
//! spacing, and grouping without restating the styling at each call site.

use iced::widget::{
    Space, button as iced_button, column, container, grid, row, text, text_input as iced_text_input,
};
use iced::{Alignment, Border, Color, Element, Fill, Length, Theme};

use super::shell;

pub(crate) fn button<'a, Message: Clone + 'a>(
    content: impl Into<Element<'a, Message>>,
) -> iced::widget::Button<'a, Message> {
    iced_button(content)
        .padding([8, 12])
        .style(shell::secondary_button)
}

pub(crate) fn text_input<'a, Message: Clone + 'a>(
    placeholder: &str,
    value: &str,
) -> iced::widget::TextInput<'a, Message> {
    iced_text_input(placeholder, value)
        .padding([10, 12])
        .style(shell::input)
}

/// A filled capsule for the one action that owns its surface.
pub(crate) fn primary_button<'a, Message: Clone + 'a>(
    content: impl Into<Element<'a, Message>>,
) -> iced::widget::Button<'a, Message> {
    iced_button(content)
        .padding([9, 16])
        .style(shell::primary_button)
}

/// A borderless glyph button, sized for the floating canvas pills.
pub(crate) fn icon_button<'a, Message: Clone + 'a>(
    glyph: &'a str,
    size: f32,
) -> iced::widget::Button<'a, Message> {
    iced_button(
        container(text(glyph).size(size))
            .width(26)
            .align_x(Alignment::Center),
    )
    .padding([6, 4])
    .style(shell::utility_button)
}

/// Names an icon-only control on hover, so a bare glyph still says what it
/// does without spending a row on a written label.
pub(crate) fn labelled<'a, Message: 'a>(
    control: impl Into<Element<'a, Message>>,
    label: impl text::IntoFragment<'a>,
) -> Element<'a, Message> {
    iced::widget::tooltip(
        control,
        container(text(label).size(12))
            .style(shell::tooltip)
            .padding([5, 9]),
        iced::widget::tooltip::Position::Bottom,
    )
    .gap(6)
    .into()
}

/// One segment of a segmented picker.
pub(crate) fn segment<'a, Message: Clone + 'a>(
    label: &'a str,
    selected: bool,
) -> iced::widget::Button<'a, Message> {
    iced_button(
        container(text(label).size(13))
            .width(Fill)
            .align_x(Alignment::Center),
    )
    .padding([6, 12])
    .style(shell::segment_button(selected))
    .width(Fill)
}

/// Wraps segments so the selected one reads as a raised capsule on a track.
pub(crate) fn segmented<'a, Message: 'a>(
    segments: impl IntoIterator<Item = Element<'a, Message>>,
) -> Element<'a, Message> {
    container(row(segments).spacing(2))
        .style(shell::segment_track)
        .padding(2)
        .into()
}

/// Lays equal-weight actions out in a grid that reflows to the panel width,
/// so a cluster of buttons never runs off the edge of the inspector.
pub(crate) fn action_grid<'a, Message: 'a>(
    actions: impl IntoIterator<Item = Element<'a, Message>>,
) -> Element<'a, Message> {
    grid(actions)
        .fluid(112)
        .spacing(6)
        .height(Length::Shrink)
        .into()
}

/// A small muted heading that labels a group without competing with it.
pub(crate) fn section_label<'a, Message: 'a>(
    label: impl text::IntoFragment<'a>,
) -> Element<'a, Message> {
    text(label).size(11).style(shell::muted_text).into()
}

/// A hairline rule for separating stacked groups.
pub(crate) fn rule<'a, Message: 'a>() -> Element<'a, Message> {
    container(Space::new().width(Fill).height(1))
        .style(shell::rule)
        .height(1)
        .into()
}

/// Groups related controls into a titled card so the inspector scans as
/// sections rather than one undifferentiated stack of buttons.
pub(crate) fn section<'a, Message: 'a>(
    title: impl text::IntoFragment<'a>,
    body: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    container(
        column![section_label(title), body.into()]
            .spacing(10)
            .width(Fill),
    )
    .style(shell::section_card)
    .padding(14)
    .width(Fill)
    .into()
}

/// Wraps a panel that already supplies its own heading in the same card
/// surface [`section`] uses, so every block in the inspector matches.
pub(crate) fn panel_card<'a, Message: 'a>(
    body: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    container(body.into())
        .style(shell::section_card)
        .padding(14)
        .width(Fill)
        .into()
}

/// Lays out three floating clusters so the center one stays optically
/// centered no matter how wide the left and right clusters grow.
pub(crate) fn float_row<'a, Message: 'a>(
    left: Element<'a, Message>,
    center: Element<'a, Message>,
    right: Element<'a, Message>,
) -> iced::widget::Row<'a, Message> {
    row![
        container(left).width(Fill).align_x(Alignment::Start),
        container(center).align_x(Alignment::Center),
        container(right).width(Fill).align_x(Alignment::End),
    ]
    .spacing(10)
    .align_y(Alignment::Start)
}

/// A short vertical hairline that separates groups inside a floating pill.
pub(crate) fn pill_divider<'a, Message: 'a>() -> Element<'a, Message> {
    container(Space::new().width(1).height(16))
        .style(shell::rule)
        .width(1)
        .into()
}

/// A round chip filled with a role's own color, so roles are told apart by
/// sight instead of by reading a hex string.
pub(crate) fn swatch<'a, Message: 'a>(hex: &str) -> Element<'a, Message> {
    let color = parse_hex_color(hex).unwrap_or(Color::from_rgb8(109, 124, 255));
    container(Space::new().width(10).height(10))
        .width(10)
        .height(10)
        .style(move |_: &Theme| {
            container::Style::default()
                .background(color)
                .border(Border {
                    radius: 5.0.into(),
                    ..Border::default()
                })
        })
        .into()
}

pub(crate) fn parse_hex_color(hex: &str) -> Option<Color> {
    let digits = hex.strip_prefix('#')?;
    if digits.len() != 6 {
        return None;
    }
    let channel = |range: std::ops::Range<usize>| u8::from_str_radix(digits.get(range)?, 16).ok();
    Some(Color::from_rgb8(
        channel(0..2)?,
        channel(2..4)?,
        channel(4..6)?,
    ))
}

/// A pill counter. Attention counts are colored so they pull the eye.
pub(crate) fn count_badge<'a, Message: 'a>(count: usize, attention: bool) -> Element<'a, Message> {
    container(text(count.to_string()).size(11))
        .style(if attention {
            shell::attention_badge
        } else {
            shell::badge
        })
        .padding([1, 7])
        .into()
}
