use iced::widget::{button, container, text, text_input};
use iced::{Background, Border, Color, Shadow, Theme, Vector, theme};

pub const SIDEBAR_WIDTH: f32 = 248.0;
pub const INSPECTOR_WIDTH: f32 = 360.0;

pub fn theme() -> Theme {
    Theme::custom(
        "OpenPodium",
        theme::Palette {
            background: Color::from_rgb8(246, 247, 249),
            text: Color::from_rgb8(29, 29, 31),
            primary: Color::from_rgb8(37, 99, 235),
            success: Color::from_rgb8(22, 163, 74),
            warning: Color::from_rgb8(217, 119, 6),
            danger: Color::from_rgb8(220, 38, 38),
        },
    )
}

pub fn sidebar(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.weakest.color.into()),
        text_color: Some(palette.background.base.text),
        border: Border {
            width: 0.0,
            radius: 0.0.into(),
            color: palette.background.weak.color,
        },
        shadow: Shadow::default(),
        ..container::Style::default()
    }
}

pub fn canvas(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style::default()
        .background(palette.background.base.color)
        .color(palette.background.base.text)
}

pub fn toolbar(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.weakest.color.into()),
        text_color: Some(palette.background.base.text),
        border: Border {
            width: 1.0,
            radius: 14.0.into(),
            color: palette.background.weak.color,
        },
        shadow: Shadow {
            color: Color::BLACK.scale_alpha(0.06),
            offset: Vector::new(0.0, 4.0),
            blur_radius: 18.0,
        },
        ..container::Style::default()
    }
}

pub fn control_group(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.weakest.color.into()),
        text_color: Some(palette.background.base.text),
        border: Border {
            width: 1.0,
            radius: 10.0.into(),
            color: palette.background.weak.color,
        },
        ..container::Style::default()
    }
}

pub fn inspector(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.weakest.color.into()),
        text_color: Some(palette.background.base.text),
        border: Border {
            width: 1.0,
            radius: 16.0.into(),
            color: palette.background.weak.color,
        },
        shadow: Shadow {
            color: Color::BLACK.scale_alpha(0.08),
            offset: Vector::new(0.0, 4.0),
            blur_radius: 24.0,
        },
        ..container::Style::default()
    }
}

pub fn card(theme: &Theme) -> container::Style {
    inspector(theme)
}

pub fn app_mark(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.primary.base.color.into()),
        text_color: Some(palette.primary.base.text),
        border: Border {
            radius: 14.0.into(),
            ..Border::default()
        },
        shadow: Shadow {
            color: palette.primary.base.color.scale_alpha(0.22),
            offset: Vector::new(0.0, 6.0),
            blur_radius: 18.0,
        },
        ..container::Style::default()
    }
}

pub fn primary_button(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let background = match status {
        button::Status::Hovered => palette.primary.strong.color,
        button::Status::Pressed => palette.primary.strong.color,
        button::Status::Active | button::Status::Disabled => palette.primary.base.color,
    };
    let mut style = button::Style {
        background: Some(background.into()),
        text_color: palette.primary.base.text,
        border: Border {
            radius: 10.0.into(),
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
        button::Status::Hovered | button::Status::Pressed => palette.background.weak.color,
        button::Status::Active | button::Status::Disabled => palette.background.weakest.color,
    };
    let mut style = button::Style {
        background: Some(background.into()),
        text_color: palette.background.base.text,
        border: Border {
            width: 1.0,
            radius: 9.0.into(),
            color: palette.background.weak.color,
        },
        shadow: Shadow::default(),
        ..button::Style::default()
    };
    if status == button::Status::Disabled {
        style.text_color = style.text_color.scale_alpha(0.45);
    }
    style
}

pub fn utility_button(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => palette.background.weak.color,
        button::Status::Active | button::Status::Disabled => Color::TRANSPARENT,
    };
    button::Style {
        background: Some(background.into()),
        text_color: palette.background.base.text.scale_alpha(0.72),
        border: Border {
            radius: 8.0.into(),
            ..Border::default()
        },
        shadow: Shadow::default(),
        ..button::Style::default()
    }
}

pub fn navigation_button(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let palette = theme.extended_palette();
        let mut style = utility_button(theme, status);
        if selected {
            style.background = Some(palette.primary.weak.color.into());
            style.text_color = palette.primary.weak.text;
        }
        style
    }
}

pub fn input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let palette = theme.extended_palette();
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: palette.background.weakest.color.into(),
        border: Border {
            width: if focused { 2.0 } else { 1.0 },
            radius: 10.0.into(),
            color: if focused {
                palette.primary.base.color
            } else {
                palette.background.weak.color
            },
        },
        icon: palette.background.base.text.scale_alpha(0.55),
        placeholder: palette.background.base.text.scale_alpha(0.42),
        value: palette.background.base.text,
        selection: palette.primary.weak.color,
    }
}

pub fn muted_text(theme: &Theme) -> text::Style {
    let palette = theme.extended_palette();
    text::Style {
        color: Some(palette.background.base.text.scale_alpha(0.72)),
    }
}

pub fn subtle_text(theme: &Theme) -> text::Style {
    let palette = theme.extended_palette();
    text::Style {
        color: Some(palette.background.base.text.scale_alpha(0.68)),
    }
}
