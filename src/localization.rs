//! Localized application-shell messages and locale-sensitive formatting.

use std::borrow::Cow;
use std::env;

use fluent_bundle::{FluentArgs, FluentBundle, FluentResource, FluentValue};
use unic_langid::LanguageIdentifier;

const EN_US: &str = include_str!("../locales/en-US.ftl");
const PT_BR: &str = include_str!("../locales/pt-BR.ftl");
const LOCALE_OVERRIDE: &str = "OPENPODIUM_LOCALE";
pub const LOCALE_KEY: &str = "locale";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Locale {
    #[default]
    EnUs,
    PtBr,
    PseudoRtl,
}

impl Locale {
    pub fn detect() -> Self {
        env::var(LOCALE_OVERRIDE)
            .ok()
            .or_else(sys_locale::get_locale)
            .as_deref()
            .map_or(Self::EnUs, Self::from_tag)
    }

    pub fn from_tag(tag: &str) -> Self {
        let normalized = tag.replace('_', "-").to_ascii_lowercase();
        if normalized == "ar-xb" || normalized == "qps-plocm" {
            Self::PseudoRtl
        } else if normalized == "pt" || normalized.starts_with("pt-") {
            Self::PtBr
        } else {
            Self::EnUs
        }
    }

    pub const fn tag(self) -> &'static str {
        match self {
            Self::EnUs => "en-US",
            Self::PtBr => "pt-BR",
            Self::PseudoRtl => "ar-XB",
        }
    }

    pub const fn is_rtl(self) -> bool {
        matches!(self, Self::PseudoRtl)
    }
}

pub struct Localizer {
    locale: Locale,
    primary: FluentBundle<FluentResource>,
    fallback: FluentBundle<FluentResource>,
}

impl Default for Localizer {
    fn default() -> Self {
        Self::new(Locale::detect())
    }
}

impl Localizer {
    pub fn new(locale: Locale) -> Self {
        let primary_source = match locale {
            Locale::EnUs | Locale::PseudoRtl => EN_US,
            Locale::PtBr => PT_BR,
        };
        Self {
            locale,
            primary: bundle(locale.tag(), primary_source),
            fallback: bundle(Locale::EnUs.tag(), EN_US),
        }
    }

    pub const fn locale(&self) -> Locale {
        self.locale
    }

    pub fn cycle_user_locale(&mut self) {
        *self = Self::new(match self.locale {
            Locale::EnUs => Locale::PtBr,
            Locale::PtBr | Locale::PseudoRtl => Locale::EnUs,
        });
    }

    pub fn text(&self, id: &str) -> String {
        self.format(id, None)
    }

    pub fn format(&self, id: &str, arguments: Option<&FluentArgs<'_>>) -> String {
        let formatted = format_message(&self.primary, id, arguments)
            .or_else(|| format_message(&self.fallback, id, arguments))
            .unwrap_or_else(|| format!("[{id}]"));
        if self.locale == Locale::PseudoRtl {
            pseudo_rtl(&formatted)
        } else {
            formatted
        }
    }

    pub fn count(&self, id: &str, count: usize) -> String {
        let mut arguments = FluentArgs::new();
        arguments.set("count", FluentValue::from(count as i64));
        self.format(id, Some(&arguments))
    }

    pub fn with_str(&self, id: &str, name: &'static str, value: impl Into<String>) -> String {
        let mut arguments = FluentArgs::new();
        arguments.set(name, FluentValue::from(value.into()));
        self.format(id, Some(&arguments))
    }

    pub fn number(&self, value: usize) -> String {
        let digits = value.to_string();
        let separator = match self.locale {
            Locale::EnUs | Locale::PseudoRtl => ',',
            Locale::PtBr => '.',
        };
        let mut output = String::with_capacity(digits.len() + digits.len() / 3);
        for (index, character) in digits.chars().enumerate() {
            if index > 0 && (digits.len() - index).is_multiple_of(3) {
                output.push(separator);
            }
            output.push(character);
        }
        if self.locale == Locale::PseudoRtl {
            pseudo_rtl(&output)
        } else {
            output
        }
    }
}

fn bundle(tag: &str, source: &str) -> FluentBundle<FluentResource> {
    let language = tag
        .parse::<LanguageIdentifier>()
        .expect("built-in locale tags are valid");
    let resource = FluentResource::try_new(source.to_owned())
        .unwrap_or_else(|(_, errors)| panic!("invalid built-in localization resource: {errors:?}"));
    let mut bundle = FluentBundle::new(vec![language]);
    bundle.set_use_isolating(false);
    bundle
        .add_resource(resource)
        .expect("built-in localization messages are unique");
    bundle
}

fn format_message(
    bundle: &FluentBundle<FluentResource>,
    id: &str,
    arguments: Option<&FluentArgs<'_>>,
) -> Option<String> {
    let pattern = bundle.get_message(id)?.value()?;
    let mut errors = Vec::new();
    let value: Cow<'_, str> = bundle.format_pattern(pattern, arguments, &mut errors);
    errors.is_empty().then(|| value.into_owned())
}

fn pseudo_rtl(value: &str) -> String {
    let expanded = value
        .chars()
        .map(|character| match character {
            'a' => 'à',
            'e' => 'ë',
            'i' => 'ï',
            'o' => 'ô',
            'u' => 'ü',
            'A' => 'À',
            'E' => 'Ë',
            'I' => 'Ï',
            'O' => 'Ô',
            'U' => 'Ü',
            other => other,
        })
        .collect::<String>();
    format!("\u{202e}⟦{expanded}⟧\u{202c}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_matching_is_stable_and_falls_back_to_english() {
        assert_eq!(Locale::from_tag("pt_BR.UTF-8"), Locale::PtBr);
        assert_eq!(Locale::from_tag("qps-plocm"), Locale::PseudoRtl);
        assert_eq!(Locale::from_tag("de-DE"), Locale::EnUs);
    }

    #[test]
    fn plurals_and_number_separators_follow_the_locale() {
        let english = Localizer::new(Locale::EnUs);
        let portuguese = Localizer::new(Locale::PtBr);
        assert_eq!(english.count("node-count", 1), "1 node");
        assert_eq!(english.count("node-count", 2), "2 nodes");
        assert_eq!(portuguese.count("node-count", 2), "2 nós");
        assert_eq!(english.number(1_234_567), "1,234,567");
        assert_eq!(portuguese.number(1_234_567), "1.234.567");
    }

    #[test]
    fn pseudo_locale_is_expanded_and_right_to_left_isolated() {
        let pseudo = Localizer::new(Locale::PseudoRtl).text("workspaces");
        assert!(pseudo.starts_with('\u{202e}'));
        assert!(pseudo.contains("Wôrkspàcës"));
        assert!(pseudo.ends_with('\u{202c}'));
    }

    #[test]
    fn user_locale_cycle_excludes_the_validation_only_pseudo_locale() {
        let mut localizer = Localizer::new(Locale::EnUs);
        localizer.cycle_user_locale();
        assert_eq!(localizer.locale(), Locale::PtBr);
        localizer.cycle_user_locale();
        assert_eq!(localizer.locale(), Locale::EnUs);
    }
}
