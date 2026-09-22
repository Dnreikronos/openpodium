use iced::widget::{column, container, pick_list, text, tooltip};
use iced::{Color, Element, Fill, Theme, theme};
use openpodium::presentation::{PresentationPreferences, ThemePreference};

use super::{Message, shell};
use crate::terminal::DefaultColors;

pub(super) fn terminal_colors(
    preferences: PresentationPreferences,
    desktop: theme::Mode,
) -> DefaultColors {
    let theme = theme(preferences, desktop);
    let palette = theme.extended_palette();
    let rgb = |color: Color| {
        let [r, g, b, _] = color.into_rgba8();
        alacritty_terminal::vte::ansi::Rgb { r, g, b }
    };
    DefaultColors {
        foreground: rgb(palette.background.base.text),
        background: rgb(shell::surface_color(palette)),
    }
}

pub(super) fn controls(preferences: PresentationPreferences) -> Element<'static, Message> {
    column![
        container(iced::widget::Space::new().height(1))
            .width(Fill)
            .style(shell::rule),
        text("Theme").size(11).style(shell::muted_text),
        tooltip(
            pick_list(
                ThemePreference::ALL,
                Some(preferences.theme()),
                Message::ThemeSelected
            )
            .text_size(12)
            .padding([7, 10])
            .width(Fill),
            "System follows your desktop appearance",
            tooltip::Position::Top,
        )
        .style(shell::tooltip),
    ]
    .spacing(7)
    .into()
}

pub(super) fn theme(preferences: PresentationPreferences, desktop: theme::Mode) -> Theme {
    let dark = preferences.theme().is_dark(desktop == theme::Mode::Dark);
    if preferences.high_contrast() {
        return Theme::custom(
            "OpenPodium high contrast",
            theme::Palette {
                background: if dark { Color::BLACK } else { Color::WHITE },
                text: if dark { Color::WHITE } else { Color::BLACK },
                primary: if dark {
                    Color::from_rgb8(0, 255, 255)
                } else {
                    Color::from_rgb8(0, 70, 160)
                },
                success: if dark {
                    Color::from_rgb8(0, 255, 0)
                } else {
                    Color::from_rgb8(0, 100, 0)
                },
                warning: if dark {
                    Color::from_rgb8(255, 255, 0)
                } else {
                    Color::from_rgb8(110, 70, 0)
                },
                danger: if dark {
                    Color::from_rgb8(255, 96, 96)
                } else {
                    Color::from_rgb8(160, 0, 0)
                },
            },
        );
    }
    if dark {
        Theme::custom(
            "OpenPodium Dark",
            theme::Palette {
                background: Color::from_rgb8(24, 26, 30),
                text: Color::from_rgb8(229, 232, 237),
                primary: Color::from_rgb8(80, 160, 255),
                success: Color::from_rgb8(74, 200, 120),
                warning: Color::from_rgb8(236, 177, 77),
                danger: Color::from_rgb8(248, 113, 113),
            },
        )
    } else {
        shell::theme()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::{GridSize, Model, Update};
    use openpodium::presentation::HIGH_CONTRAST_KEY;

    #[test]
    fn terminal_queries_match_rendered_colors_in_every_theme() {
        let mut preferences = PresentationPreferences::default();
        let mut model = Model::new(GridSize {
            columns: 40,
            rows: 12,
        });
        for contrast in ["false", "true"] {
            preferences.apply_stored([(HIGH_CONTRAST_KEY.to_owned(), contrast.to_owned())]);
            for selected in ThemePreference::ALL {
                preferences.set_theme(selected);
                for desktop in [theme::Mode::Light, theme::Mode::Dark] {
                    let theme = theme(preferences, desktop);
                    let palette = theme.extended_palette();
                    let colors = terminal_colors(preferences, desktop);
                    for (query, rendered) in [
                        (10, palette.background.base.text),
                        (11, shell::surface_color(palette)),
                        (12, palette.background.base.text),
                    ] {
                        let [r, g, b, _] = rendered.into_rgba8();
                        let expected = format!("rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}");
                        let updates = model
                            .feed_with_colors(format!("\x1b]{query};?\x07").as_bytes(), colors);
                        assert!(updates.iter().any(|update| matches!(update,
                            Update::PtyWrite(bytes) if String::from_utf8_lossy(bytes).contains(&expected)
                        )), "{selected:?} / {desktop:?} OSC {query} must report {expected}");
                    }
                }
            }
        }
    }

    #[test]
    fn both_palettes_follow_the_selected_mode_even_with_high_contrast() {
        let mut preferences = PresentationPreferences::default();
        for contrast in ["false", "true"] {
            preferences.apply_stored([(HIGH_CONTRAST_KEY.to_owned(), contrast.to_owned())]);
            for selected in ThemePreference::ALL {
                preferences.set_theme(selected);
                for desktop in [theme::Mode::Light, theme::Mode::Dark] {
                    let palette = theme(preferences, desktop);
                    assert_eq!(
                        palette.extended_palette().is_dark,
                        selected.is_dark(desktop == theme::Mode::Dark)
                    );
                }
            }
        }
    }
}
