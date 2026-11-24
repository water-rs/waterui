//! Runtime translation catalog loaded from app-provided i18n tables.

use core::cmp::Ordering;
use std::collections::BTreeMap;

use icu_locale::LanguageIdentifier;
use suiteki::Str;
use waterui_core::contract_violation;
use waterui_core::plugin::Plugin;

use crate::locale::{Locale, find_in_fallback_chain, locales};
use crate::parser::{TranslationFile, TranslationParseError, TranslationValue};

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocaleKey(LanguageIdentifier);

impl Ord for LocaleKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl PartialOrd for LocaleKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Runtime translation catalog installed into the environment.
#[derive(Debug, Clone, Default)]
pub struct TranslationCatalog {
    locales: BTreeMap<LocaleKey, (Locale, TranslationFile)>,
}

impl TranslationCatalog {
    /// Creates an empty translation catalog.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses and inserts a locale TOML document.
    ///
    /// # Errors
    ///
    /// Returns [`TranslationParseError`] if the TOML content or schema is invalid.
    pub fn add_toml(self, locale: Locale, content: &str) -> Result<Self, TranslationParseError> {
        let file = TranslationFile::parse(content)?;
        Ok(self.insert_file(locale, file))
    }

    /// Inserts a parsed translation file for a locale.
    #[must_use]
    pub fn insert_file(mut self, locale: Locale, file: TranslationFile) -> Self {
        self.locales
            .insert(LocaleKey(locale.id().clone()), (locale, file));
        self
    }

    /// Iterates over every locale registered in this catalog in deterministic order.
    pub fn locales(&self) -> impl Iterator<Item = &Locale> {
        self.locales.values().map(|(locale, _)| locale)
    }

    /// Resolves a simple translation key for a locale.
    ///
    /// If the matching entry is a plural or dual-plural table, this reports a
    /// contract violation (panicking in debug builds, logging in release builds)
    /// and returns its `other` / `other_other` form.
    #[must_use]
    pub fn lookup_text(&self, locale: &Locale, key: &str) -> Option<Str> {
        find_in_fallback_chain(locale, |fallback| self.lookup_exact(fallback.id(), key))
            .or_else(|| self.lookup_exact(locales::EN.id(), key))
    }

    fn lookup_exact(&self, locale_key: &LanguageIdentifier, key: &str) -> Option<Str> {
        let (_, file) = self.locales.get(&LocaleKey(locale_key.clone()))?;
        let value = file.get(key)?;
        match value {
            TranslationValue::Simple(text) => Some(Str::from(text.clone())),
            TranslationValue::Plural(forms) => {
                contract_violation!(
                    "TranslationCatalog::lookup_text resolved pluralized key '{key}' for locale '{locale_key}'; use text! for pluralized strings; falling back to the 'other' form"
                );
                Some(Str::from(forms.other.clone()))
            }
            TranslationValue::DualPlural(forms) => {
                contract_violation!(
                    "TranslationCatalog::lookup_text resolved dual-plural key '{key}' for locale '{locale_key}'; use text! for pluralized strings; falling back to the 'other_other' form"
                );
                Some(Str::from(forms.other_other().to_owned()))
            }
            _ => None,
        }
    }
}

impl Plugin for TranslationCatalog {}

#[cfg(test)]
mod tests {
    use super::*;

    const GREETING: &str = r#"hello = "Hello""#;

    #[test]
    fn locales_enumerates_registered_locales_in_deterministic_order() {
        let catalog = TranslationCatalog::new()
            .add_toml(locales::JA, GREETING)
            .expect("ja catalog must parse")
            .add_toml(locales::EN_US, GREETING)
            .expect("en-US catalog must parse")
            .add_toml(locales::DE, GREETING)
            .expect("de catalog must parse");

        let tags: Vec<String> = catalog.locales().map(Locale::canonical_tag).collect();
        assert_eq!(tags, vec!["de", "en-US", "ja"]);
    }

    #[test]
    fn add_toml_rejects_empty_plural_other() {
        let err = TranslationCatalog::new()
            .add_toml(locales::EN, r#"apples = { one = "an apple", other = "" }"#)
            .expect_err("empty 'other' must fail");
        assert!(matches!(
            err,
            TranslationParseError::MissingPluralOther { .. }
        ));
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "resolved pluralized key 'apples'")
    )]
    fn looking_up_a_pluralized_key_falls_back_to_other_form() {
        let catalog = TranslationCatalog::new()
            .add_toml(
                locales::EN,
                r#"apples = { one = "an apple", other = "many apples" }"#,
            )
            .expect("the table is valid TOML");

        assert_eq!(
            catalog.lookup_text(&locales::EN, "apples").as_deref(),
            Some("many apples")
        );
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "resolved dual-plural key 'fruit'")
    )]
    fn looking_up_a_dual_plural_key_falls_back_to_other_other_form() {
        let catalog = TranslationCatalog::new()
            .add_toml(
                locales::EN,
                r#"fruit = { one_one = "1 and 1", other_other = "many fruits" }"#,
            )
            .expect("the table is valid TOML");

        assert_eq!(
            catalog.lookup_text(&locales::EN, "fruit").as_deref(),
            Some("many fruits")
        );
    }

    #[test]
    fn looking_up_a_simple_key_finds_its_text() {
        let catalog = TranslationCatalog::new()
            .add_toml(locales::EN, GREETING)
            .expect("the table is valid TOML");

        assert_eq!(
            catalog.lookup_text(&locales::EN, "hello").as_deref(),
            Some("Hello")
        );
    }
}
