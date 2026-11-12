//! Runtime translation catalog loaded from app-provided i18n tables.

use std::collections::HashMap;
use std::string::String;

use icu_locale::LanguageIdentifier;
use suiteki::Str;
use waterui_core::contract_violation;
use waterui_core::plugin::Plugin;

use crate::locale::{Locale, find_in_fallback_chain, locales};
use crate::parser::{TranslationFile, TranslationValue};

/// Runtime translation catalog installed into the environment.
#[derive(Debug, Clone, Default)]
pub struct TranslationCatalog {
    locales: HashMap<LanguageIdentifier, TranslationFile>,
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
    /// Returns an error if the TOML content is invalid.
    pub fn add_toml(
        mut self,
        locale: impl Into<String>,
        content: &str,
    ) -> Result<Self, toml::de::Error> {
        let file = TranslationFile::parse(content)?;
        let locale = locale.into();
        self.insert_locale(&locale, file);
        Ok(self)
    }

    /// Inserts a parsed translation file for a locale.
    #[must_use]
    pub fn insert_file(mut self, locale: impl Into<String>, file: TranslationFile) -> Self {
        let locale = locale.into();
        self.insert_locale(&locale, file);
        self
    }

    /// Resolves a simple translation key for a locale.
    #[must_use]
    pub fn lookup_text(&self, locale: &Locale, key: &str) -> Option<Str> {
        find_in_fallback_chain(locale, |fallback| self.lookup_exact(fallback.id(), key))
            .or_else(|| self.lookup_exact(locales::EN.id(), key))
    }

    fn insert_locale(&mut self, locale: &str, file: TranslationFile) {
        let parsed = match locale.parse::<Locale>() {
            Ok(parsed) => parsed,
            Err(error) => {
                contract_violation!(
                    "TranslationCatalog locale '{locale}' is invalid: {error}; the translations for it are not added"
                );
                return;
            }
        };
        self.locales.insert(parsed.id().clone(), file);
    }

    fn lookup_exact(&self, locale_key: &LanguageIdentifier, key: &str) -> Option<Str> {
        let file = self.locales.get(locale_key)?;
        let value = file.get(key)?;
        match value {
            TranslationValue::Simple(text) => Some(Str::from(text.clone())),
            TranslationValue::Plural(_forms) => {
                contract_violation!(
                    "TranslationCatalog::lookup_text cannot resolve pluralized key '{key}' for locale '{locale_key}'; use text! for pluralized strings; the lookup finds no text"
                );
                None
            }
            TranslationValue::DualPlural(_) => {
                contract_violation!(
                    "TranslationCatalog::lookup_text cannot resolve dual-plural key '{key}' for locale '{locale_key}'; use text! for pluralized strings; the lookup finds no text"
                );
                None
            }
        }
    }
}

impl Plugin for TranslationCatalog {}

#[cfg(test)]
mod tests {
    use super::*;

    const GREETING: &str = r#"hello = "Hello""#;

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "TranslationCatalog locale 'not a locale!' is invalid")
    )]
    fn an_invalid_locale_is_not_added() {
        let catalog = TranslationCatalog::new()
            .add_toml("not a locale!", GREETING)
            .expect("the table is valid TOML");

        assert!(catalog.locales.is_empty());
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "cannot resolve pluralized key 'apples'")
    )]
    fn looking_up_a_pluralized_key_finds_no_text() {
        let catalog = TranslationCatalog::new()
            .add_toml(
                "en",
                r#"apples = { one = "an apple", other = "many apples" }"#,
            )
            .expect("the table is valid TOML");

        assert_eq!(catalog.lookup_text(&locales::EN, "apples"), None);
    }

    #[test]
    fn looking_up_a_simple_key_finds_its_text() {
        let catalog = TranslationCatalog::new()
            .add_toml("en", GREETING)
            .expect("the table is valid TOML");

        assert_eq!(
            catalog.lookup_text(&locales::EN, "hello").as_deref(),
            Some("Hello")
        );
    }
}
