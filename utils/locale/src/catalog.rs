//! Runtime translation catalog loaded from app-provided i18n tables.

use std::collections::BTreeMap;
use std::string::{String, ToString};

use waterui_core::plugin::Plugin;
use waterui_str::Str;

use crate::locale::{Locale, get_fallback_chain};
use crate::parser::{TranslationFile, TranslationValue};

/// Runtime translation catalog installed into the environment.
#[derive(Debug, Clone, Default)]
pub struct TranslationCatalog {
    locales: BTreeMap<String, TranslationFile>,
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
        self.locales.insert(locale.into(), file);
        Ok(self)
    }

    /// Inserts a parsed translation file for a locale.
    #[must_use]
    pub fn insert_file(mut self, locale: impl Into<String>, file: TranslationFile) -> Self {
        self.locales.insert(locale.into(), file);
        self
    }

    /// Resolves a simple translation key for a locale.
    #[must_use]
    pub fn lookup_text(&self, locale: &Locale, key: &str) -> Option<Str> {
        for fallback in get_fallback_chain(locale) {
            if let Some(text) = self.lookup_exact(fallback.id().to_string().as_str(), key) {
                return Some(text);
            }
        }

        self.lookup_exact("en", key)
    }

    fn lookup_exact(&self, locale_key: &str, key: &str) -> Option<Str> {
        let file = self.locales.get(locale_key)?;
        let value = file.get(key)?;
        match value {
            TranslationValue::Simple(text) => Some(Str::from(text.clone())),
            TranslationValue::Plural(_forms) => {
                panic!(
                    "TranslationCatalog::lookup_text cannot resolve pluralized key '{key}' for locale '{locale_key}'; use text! for pluralized strings"
                );
            }
            TranslationValue::DualPlural(_) => {
                panic!(
                    "TranslationCatalog::lookup_text cannot resolve dual-plural key '{key}' for locale '{locale_key}'; use text! for pluralized strings"
                );
            }
        }
    }
}

impl Plugin for TranslationCatalog {}
