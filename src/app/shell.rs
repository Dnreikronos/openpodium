//! The OpenPodium design system: the surfaces, controls, and layout helpers
//! every panel is built from, so the whole application reads as one product
//! rather than a pile of default widgets.

use iced::theme::palette;
use iced::widget::{button, container, text, text_input};
use iced::{Background, Border, Color, Shadow, Theme, Vector, theme};

pub const SIDEBAR_WIDTH: f32 = 212.0;
pub const STAGE_PADDING: f32 = 4.0;

/// Radius large enough to render any control height as a capsule.
const CAPSULE: f32 = 999.0;

pub fn theme() -> Theme {
    Theme::custom(
        "OpenPodium",
        theme::Palette {
            background: Color::from_rgb8(248, 249, 250),
            text: Color::from_rgb8(39, 42, 47),
            primary: Color::from_rgb8(10, 132, 255),
            success: Color::from_rgb8(22, 163, 74),
            warning: Color::from_rgb8(217, 119, 6),
            danger: Color::from_rgb8(220, 38, 38),
        },
    )
}

/// Border color for the hairline rules that separate calm surfaces.
///
/// High contrast themes replace the near-invisible neutral with the text color
/// so that every surface edge stays perceivable.
fn hairline(theme: &Theme) -> Color {
    hairline_color(theme.extended_palette())
}

/// The palette-level form, for the canvas renderer which draws from a palette
/// rather than a theme.
pub(crate) fn hairline_color(palette: &palette::Extended) -> Color {
    if palette.is_dark {
        palette.background.base.text.scale_alpha(0.7)
    } else {
        Color::from_rgb8(222, 225, 229)
    }
}

/// The raised surface used by cards, pills, and panels.
fn surface(theme: &Theme) -> Color {
    surface_color(theme.extended_palette())
}

pub(crate) fn surface_color(palette: &palette::Extended) -> Color {
    if palette.is_dark {
        palette.background.base.color
    } else {
        Color::WHITE
    }
}

/// The recessed neutral for tracks, badges, key caps, and hover fills.
fn sunken(theme: &Theme) -> Color {
    sunken_color(theme.extended_palette())
}

pub(crate) fn sunken_color(palette: &palette::Extended) -> Color {
    if palette.is_dark {
        palette.background.weak.color
    } else {
        Color::from_rgb8(237, 239, 241)
    }
}

/// The fill that marks a selected row. A shade stronger than [`sunken`] so it
/// still reads as chosen when it sits on the workspace rail rather than on a
/// white card.
fn selected_fill(theme: &Theme) -> Color {
    let palette = theme.extended_palette();
    if palette.is_dark {
        palette.background.strong.color
    } else {
        Color::from_rgb8(223, 226, 230)
    }
}

/// The quiet grey the window chrome is built from: the workspace rail and the
/// backdrop the canvas sheet floats on. Iced's derived `weak` neutral is too
/// saturated to sit behind a whole window, so the light value is explicit.
fn chrome(theme: &Theme) -> Color {
    chrome_color(theme.extended_palette())
}

pub(crate) fn chrome_color(palette: &palette::Extended) -> Color {
    if palette.is_dark {
        palette.background.base.color
    } else {
        Color::from_rgb8(248, 249, 250)
    }
}

fn soft_shadow(theme: &Theme, blur: f32) -> Shadow {
    if theme.extended_palette().is_dark {
        Shadow::default()
    } else {
        Shadow {
            color: Color::BLACK.scale_alpha(0.06),
            offset: Vector::new(0.0, blur * 0.15),
            blur_radius: blur,
        }
    }
}

pub fn sidebar(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(chrome(theme).into()),
        text_color: Some(palette.background.base.text),
        border: Border {
            width: 0.0,
            radius: 0.0.into(),
            color: hairline(theme),
        },
        shadow: Shadow::default(),
        ..container::Style::default()
    }
}

/// Backdrop behind the canvas and its floating controls.
pub fn canvas(theme: &Theme) -> container::Style {
    container::Style::default()
        .background(chrome(theme))
        .color(theme.extended_palette().background.base.text)
}

/// The canvas sheet itself: a calm surface the floating pills sit on top of.
pub fn canvas_surface(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(surface(theme).into()),
        text_color: Some(theme.extended_palette().background.base.text),
        border: Border {
            width: if theme.extended_palette().is_dark {
                1.0
            } else {
                0.0
            },
            radius: 8.0.into(),
            color: hairline(theme),
        },
        shadow: Shadow::default(),
        ..container::Style::default()
    }
}

/// A control cluster that floats over the canvas.
pub fn floating_pill(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(surface(theme).into()),
        text_color: Some(theme.extended_palette().background.base.text),
        border: Border {
            width: 1.0,
            radius: CAPSULE.into(),
            color: hairline(theme).scale_alpha(if theme.extended_palette().is_dark {
                1.0
            } else {
                0.55
            }),
        },
        shadow: soft_shadow(theme, 12.0),
        ..container::Style::default()
    }
}

/// A floating label, such as the workspace name over the canvas.
pub fn floating_chip(theme: &Theme) -> container::Style {
    container::Style {
        border: Border {
            radius: CAPSULE.into(),
            ..floating_pill(theme).border
        },
        shadow: Shadow::default(),
        ..floating_pill(theme)
    }
}

/// The recessed track behind a segmented control.
pub fn segment_track(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(sunken(theme).into()),
        text_color: Some(palette.background.base.text),
        border: Border {
            width: if palette.is_dark { 1.0 } else { 0.0 },
            radius: 11.0.into(),
            color: hairline(theme),
        },
        ..container::Style::default()
    }
}

fn dialog_surface(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(surface(theme).into()),
        text_color: Some(theme.extended_palette().background.base.text),
        border: Border {
            width: 1.0,
            radius: 10.0.into(),
            color: hairline(theme),
        },
        shadow: Shadow::default(),
        ..container::Style::default()
    }
}

/// A grouped block of related controls inside a dialog.
pub fn section_card(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(
            if palette.is_dark {
                palette.background.base.color
            } else {
                Color::from_rgb8(250, 251, 252)
            }
            .into(),
        ),
        text_color: Some(palette.background.base.text),
        border: Border {
            width: 1.0,
            radius: 8.0.into(),
            color: hairline(theme),
        },
        shadow: Shadow::default(),
        ..container::Style::default()
    }
}

pub fn card(theme: &Theme) -> container::Style {
    container::Style {
        shadow: soft_shadow(theme, 28.0),
        ..dialog_surface(theme)
    }
}

/// A one pixel rule. Render inside a container with a fixed height of 1.
pub fn rule(theme: &Theme) -> container::Style {
    container::Style::default().background(hairline(theme))
}

pub fn badge(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(sunken(theme).into()),
        text_color: Some(palette.background.base.text.scale_alpha(0.62)),
        border: Border {
            radius: CAPSULE.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// A counter that must pull the eye, such as unread agent attention.
pub fn attention_badge(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.danger.base.color.into()),
        text_color: Some(palette.danger.base.text),
        border: Border {
            radius: CAPSULE.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

pub fn app_mark(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(sunken(theme).into()),
        text_color: Some(palette.primary.base.color),
        border: Border {
            radius: 8.0.into(),
            ..Border::default()
        },
        shadow: Shadow::default(),
        ..container::Style::default()
    }
}

pub fn primary_button(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => palette.primary.strong.color,
        button::Status::Active | button::Status::Disabled => palette.primary.base.color,
    };
    let mut style = button::Style {
        background: Some(background.into()),
        text_color: palette.primary.base.text,
        border: Border {
            radius: CAPSULE.into(),
            ..Border::default()
        },
        shadow: Shadow::default(),
        ..button::Style::default()
    };
    if status == button::Status::Disabled {
        style.background = style
            .background
            .map(|_| Background::Color(background.scale_alpha(0.45)));
        style.text_color = style.text_color.scale_alpha(0.65);
    }
    style
}

pub fn secondary_button(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => sunken(theme),
        button::Status::Active | button::Status::Disabled => surface(theme),
    };
    let mut style = button::Style {
        background: Some(background.into()),
        text_color: palette.background.base.text,
        border: Border {
            width: 1.0,
            radius: 8.0.into(),
            color: hairline(theme),
        },
        shadow: Shadow::default(),
        ..button::Style::default()
    };
    if status == button::Status::Disabled {
        style.text_color = style.text_color.scale_alpha(0.45);
    }
    style
}

/// A destructive action. Quiet until hovered, then unmistakably red.
pub fn danger_button(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let mut style = secondary_button(theme, status);
    style.text_color = palette.danger.base.color;
    if matches!(status, button::Status::Hovered | button::Status::Pressed) {
        style.background = Some(palette.danger.base.color.into());
        style.text_color = palette.danger.base.text;
        style.border.color = palette.danger.base.color;
    }
    style
}

pub fn utility_button(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => sunken(theme),
        button::Status::Active | button::Status::Disabled => Color::TRANSPARENT,
    };
    button::Style {
        background: Some(background.into()),
        text_color: palette.background.base.text.scale_alpha(0.72),
        border: Border {
            radius: CAPSULE.into(),
            ..Border::default()
        },
        shadow: Shadow::default(),
        ..button::Style::default()
    }
}

/// One segment of a segmented control. The selected segment rides as a raised
/// capsule on the recessed track, the way native tab pickers read.
pub fn segment_button(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let palette = theme.extended_palette();
        if selected {
            return button::Style {
                background: Some(palette.primary.base.color.into()),
                text_color: palette.primary.base.text,
                border: Border {
                    radius: 9.0.into(),
                    ..Border::default()
                },
                shadow: Shadow::default(),
                ..button::Style::default()
            };
        }
        let background = match status {
            button::Status::Hovered | button::Status::Pressed => surface(theme).scale_alpha(0.75),
            button::Status::Active | button::Status::Disabled => Color::TRANSPARENT,
        };
        button::Style {
            background: Some(background.into()),
            text_color: palette.background.base.text.scale_alpha(0.68),
            border: Border {
                radius: 9.0.into(),
                ..Border::default()
            },
            shadow: Shadow::default(),
            ..button::Style::default()
        }
    }
}

pub fn navigation_button(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let palette = theme.extended_palette();
        let mut style = utility_button(theme, status);
        style.border.radius = 9.0.into();
        style.text_color = palette.background.base.text;
        if selected {
            style.background = Some(selected_fill(theme).into());
        }
        style
    }
}

pub fn input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let palette = theme.extended_palette();
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: surface(theme).into(),
        border: Border {
            width: if focused { 2.0 } else { 1.0 },
            radius: 9.0.into(),
            color: if focused {
                palette.primary.base.color
            } else {
                hairline(theme)
            },
        },
        icon: palette.background.base.text.scale_alpha(0.55),
        placeholder: palette.background.base.text.scale_alpha(0.42),
        value: palette.background.base.text,
        selection: palette.primary.weak.color,
    }
}

/// A borderless field for a search surface that is already a card.
pub fn search_input(theme: &Theme, _status: text_input::Status) -> text_input::Style {
    let palette = theme.extended_palette();
    text_input::Style {
        background: Color::TRANSPARENT.into(),
        border: Border::default(),
        icon: palette.background.base.text.scale_alpha(0.55),
        placeholder: palette.background.base.text.scale_alpha(0.42),
        value: palette.background.base.text,
        selection: palette.primary.weak.color,
    }
}

/// Dims whatever sits behind a modal surface.
pub fn scrim(_theme: &Theme) -> container::Style {
    container::Style::default().background(Color::BLACK.scale_alpha(0.28))
}

/// The label that names an icon-only control on hover.
pub fn tooltip(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(surface(theme).into()),
        text_color: Some(palette.background.base.text),
        border: Border {
            width: 1.0,
            radius: 7.0.into(),
            color: hairline(theme),
        },
        shadow: soft_shadow(theme, 16.0),
        ..container::Style::default()
    }
}

/// A recessed field. Used where a control should read as something you type
/// into, even when it opens a palette instead.
pub fn field(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(sunken(theme).into()),
        text_color: Some(palette.background.base.text),
        border: Border {
            width: if palette.is_dark { 1.0 } else { 0.0 },
            radius: 8.0.into(),
            color: hairline(theme),
        },
        ..container::Style::default()
    }
}

/// A keyboard shortcut rendered as a key cap.
pub fn key_cap(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(sunken(theme).into()),
        text_color: Some(palette.background.base.text.scale_alpha(0.7)),
        border: Border {
            width: 1.0,
            radius: 5.0.into(),
            color: hairline(theme),
        },
        ..container::Style::default()
    }
}

/// Secondary text. High contrast themes barely fade it, because there the
/// whole point of the palette is that nothing recedes out of legibility.
pub(crate) fn muted_color(palette: &palette::Extended) -> Color {
    palette
        .background
        .base
        .text
        .scale_alpha(if palette.is_dark { 0.9 } else { 0.72 })
}

/// Tertiary text: paths, hints, and other detail read after the label.
pub(crate) fn subtle_color(palette: &palette::Extended) -> Color {
    palette
        .background
        .base
        .text
        .scale_alpha(if palette.is_dark { 0.85 } else { 0.68 })
}

pub fn muted_text(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(muted_color(theme.extended_palette())),
    }
}

pub fn subtle_text(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(subtle_color(theme.extended_palette())),
    }
}
