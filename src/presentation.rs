//! User presentation preferences that remain independent of the GUI toolkit.

use std::{env, time::Duration};

use mundy::{Contrast, Interest, Preferences as SystemPreferences};

const TEXT_SCALE_ENV: &str = "OPENPODIUM_TEXT_SCALE";
const HIGH_CONTRAST_ENV: &str = "OPENPODIUM_HIGH_CONTRAST";
const REDUCED_MOTION_ENV: &str = "OPENPODIUM_REDUCED_MOTION";
const SYSTEM_PREFERENCES_TIMEOUT: Duration = Duration::from_millis(200);
const TEXT_SCALES: [f32; 5] = [1.0, 1.25, 1.5, 1.75, 2.0];
pub const TEXT_SCALE_KEY: &str = "text_scale";
pub const HIGH_CONTRAST_KEY: &str = "high_contrast";
pub const REDUCED_MOTION_KEY: &str = "reduced_motion";
pub const THEME_KEY: &str = "theme";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemePreference {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn from_stored(value: &str) -> Self {
        match value {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::System,
        }
    }

    pub const fn is_dark(self, desktop_is_dark: bool) -> bool {
        match self {
            Self::System => desktop_is_dark,
            Self::Light => false,
            Self::Dark => true,
        }
    }
}

impl std::fmt::Display for ThemePreference {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PresentationPreferences {
    scale_index: usize,
    high_contrast: bool,
    reduced_motion: bool,
    theme: ThemePreference,
}

impl Default for PresentationPreferences {
    fn default() -> Self {
        Self::from_environment()
    }
}

impl PresentationPreferences {
    pub fn from_environment() -> Self {
        let system = system_preferences();
        let requested_scale = env::var(TEXT_SCALE_ENV)
            .ok()
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(1.0);
        Self {
            scale_index: closest_scale_index(requested_scale),
            high_contrast: env_flag(HIGH_CONTRAST_ENV).unwrap_or(system.high_contrast),
            reduced_motion: env_flag(REDUCED_MOTION_ENV).unwrap_or(system.reduced_motion),
            theme: ThemePreference::System,
        }
    }

    pub const fn text_scale(self) -> f32 {
        TEXT_SCALES[self.scale_index]
    }

    pub fn text_scale_percent(self) -> u16 {
        (self.text_scale() * 100.0).round() as u16
    }

    pub const fn high_contrast(self) -> bool {
        self.high_contrast
    }

    pub const fn reduced_motion(self) -> bool {
        self.reduced_motion
    }

    pub const fn theme(self) -> ThemePreference {
        self.theme
    }

    pub fn set_theme(&mut self, theme: ThemePreference) {
        self.theme = theme;
    }

    pub fn cycle_text_scale(&mut self) {
        self.scale_index = (self.scale_index + 1) % TEXT_SCALES.len();
    }

    pub fn toggle_high_contrast(&mut self) {
        self.high_contrast = !self.high_contrast;
    }

    pub fn toggle_reduced_motion(&mut self) {
        self.reduced_motion = !self.reduced_motion;
    }

    pub fn apply_stored(&mut self, values: impl IntoIterator<Item = (String, String)>) {
        for (key, value) in values {
            match key.as_str() {
                THEME_KEY => self.theme = ThemePreference::from_stored(&value),
                TEXT_SCALE_KEY => {
                    if let Ok(requested) = value.parse::<f32>() {
                        self.scale_index = closest_scale_index(requested);
                    }
                }
                HIGH_CONTRAST_KEY => self.high_contrast = stored_flag(&value),
                REDUCED_MOTION_KEY => self.reduced_motion = stored_flag(&value),
                _ => {}
            }
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct SystemPresentationPreferences {
    high_contrast: bool,
    reduced_motion: bool,
}

fn system_preferences() -> SystemPresentationPreferences {
    // On macOS, mundy requires this query to run on the main thread. Keeping
    // the fallback here also makes PresentationPreferences safe to construct
    // from worker-thread tests and background tooling.
    #[cfg(target_os = "macos")]
    if std::thread::current().name() != Some("main") {
        return SystemPresentationPreferences::default();
    }

    SystemPreferences::once_blocking(
        Interest::Contrast | Interest::ReducedMotion,
        SYSTEM_PREFERENCES_TIMEOUT,
    )
    .map_or_else(SystemPresentationPreferences::default, |preferences| {
        SystemPresentationPreferences {
            high_contrast: matches!(preferences.contrast, Contrast::More | Contrast::Custom),
            reduced_motion: preferences.reduced_motion.is_reduce(),
        }
    })
}

fn env_flag(name: &str) -> Option<bool> {
    env::var(name).ok().and_then(|value| parse_flag(&value))
}

fn parse_flag(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn stored_flag(value: &str) -> bool {
    matches!(value, "true" | "1")
}

fn closest_scale_index(requested: f32) -> usize {
    let requested = requested.clamp(TEXT_SCALES[0], *TEXT_SCALES.last().expect("scales exist"));
    TEXT_SCALES
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| {
            (*left - requested)
                .abs()
                .total_cmp(&(*right - requested).abs())
        })
        .map_or(0, |(index, _)| index)
}

pub fn contrast_ratio(foreground: [u8; 3], background: [u8; 3]) -> f64 {
    let foreground = luminance(foreground);
    let background = luminance(background);
    let (lighter, darker) = if foreground > background {
        (foreground, background)
    } else {
        (background, foreground)
    };
    (lighter + 0.05) / (darker + 0.05)
}

fn luminance(color: [u8; 3]) -> f64 {
    let channel = |value: u8| {
        let value = f64::from(value) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(color[0]) + 0.7152 * channel(color[1]) + 0.0722 * channel(color[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_cycles_only_through_supported_bounds() {
        let mut preferences = PresentationPreferences {
            scale_index: 0,
            high_contrast: false,
            reduced_motion: false,
            theme: ThemePreference::System,
        };
        for expected in [125, 150, 175, 200, 100] {
            preferences.cycle_text_scale();
            assert_eq!(preferences.text_scale_percent(), expected);
        }
    }

    #[test]
    fn high_contrast_palette_exceeds_enhanced_text_requirement() {
        assert!(contrast_ratio([255, 255, 255], [0, 0, 0]) >= 7.0);
        assert!(contrast_ratio([0, 255, 255], [0, 0, 0]) >= 7.0);
        assert!(contrast_ratio([255, 255, 0], [0, 0, 0]) >= 7.0);
    }

    #[test]
    fn stored_preferences_override_defaults_and_ignore_unknown_keys() {
        let mut preferences = PresentationPreferences {
            scale_index: 0,
            high_contrast: false,
            reduced_motion: false,
            theme: ThemePreference::System,
        };
        preferences.apply_stored([
            (TEXT_SCALE_KEY.to_owned(), "1.8".to_owned()),
            (HIGH_CONTRAST_KEY.to_owned(), "true".to_owned()),
            (REDUCED_MOTION_KEY.to_owned(), "1".to_owned()),
            ("future_preference".to_owned(), "kept elsewhere".to_owned()),
        ]);
        assert_eq!(preferences.text_scale_percent(), 175);
        assert!(preferences.high_contrast());
        assert!(preferences.reduced_motion());
    }

    #[test]
    fn environment_flags_support_explicit_enable_and_disable_values() {
        for value in ["1", "true", "YES", " on "] {
            assert_eq!(parse_flag(value), Some(true));
        }
        for value in ["0", "false", "NO", " off "] {
            assert_eq!(parse_flag(value), Some(false));
        }
        assert_eq!(parse_flag("sometimes"), None);
    }

    #[test]
    fn theme_preferences_round_trip_and_follow_system_only_when_selected() {
        for theme in ThemePreference::ALL {
            assert_eq!(ThemePreference::from_stored(theme.as_str()), theme);
        }
        assert_eq!(
            ThemePreference::from_stored("invalid"),
            ThemePreference::System
        );
        for desktop in [false, true] {
            assert_eq!(ThemePreference::System.is_dark(desktop), desktop);
            assert!(!ThemePreference::Light.is_dark(desktop));
            assert!(ThemePreference::Dark.is_dark(desktop));
        }
        let mut preferences = PresentationPreferences::default();
        preferences.apply_stored([(THEME_KEY.to_owned(), "dark".to_owned())]);
        assert_eq!(preferences.theme(), ThemePreference::Dark);
    }
}
