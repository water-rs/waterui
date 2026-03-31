//! TOML translation file parser.

use alloc::collections::BTreeMap;
use alloc::string::String;
use core::fmt;

use serde::Deserialize;

use crate::plural::PluralCategory;

extern crate alloc;

/// Single plural source translation.
///
/// # TOML Format
///
/// ```toml
/// "I have {#count} apple" = {
///   one = "I have {count} apple",
///   other = "I have {count} apples"
/// }
/// ```
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluralForms {
    /// Form for zero items (some languages)
    #[serde(default)]
    pub zero: Option<String>,

    /// Form for one item
    #[serde(default)]
    pub one: Option<String>,

    /// Form for two items (some languages)
    #[serde(default)]
    pub two: Option<String>,

    /// Form for few items (some languages)
    #[serde(default)]
    pub few: Option<String>,

    /// Form for many items (some languages)
    #[serde(default)]
    pub many: Option<String>,

    /// Fallback form (required)
    pub other: String,
}

impl PluralForms {
    /// Get the translation for a plural category.
    #[must_use]
    pub fn get(&self, cat: PluralCategory) -> &str {
        match cat {
            PluralCategory::Zero => self.zero.as_deref().unwrap_or(&self.other),
            PluralCategory::One => self.one.as_deref().unwrap_or(&self.other),
            PluralCategory::Two => self.two.as_deref().unwrap_or(&self.other),
            PluralCategory::Few => self.few.as_deref().unwrap_or(&self.other),
            PluralCategory::Many => self.many.as_deref().unwrap_or(&self.other),
            PluralCategory::Other => &self.other,
        }
    }

    /// Iterate over all defined categories.
    pub fn iter_categories(&self) -> impl Iterator<Item = (PluralCategory, Option<&str>)> {
        [
            (PluralCategory::Zero, self.zero.as_deref()),
            (PluralCategory::One, self.one.as_deref()),
            (PluralCategory::Two, self.two.as_deref()),
            (PluralCategory::Few, self.few.as_deref()),
            (PluralCategory::Many, self.many.as_deref()),
            (PluralCategory::Other, Some(self.other.as_str())),
        ]
        .into_iter()
    }
}

/// Dual plural source translation (e.g., apples + oranges).
///
/// # TOML Format
///
/// ```toml
/// "I have {#apples} apple and {#oranges} orange" = {
///   one_one = "I have {apples} apple and {oranges} orange",
///   one_other = "I have {apples} apple and {oranges} oranges",
///   other_one = "I have {apples} apples and {oranges} orange",
///   other_other = "I have {apples} apples and {oranges} oranges"
/// }
/// ```
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DualPluralForms {
    /// one + one
    #[serde(default)]
    pub one_one: Option<String>,

    /// one + other
    #[serde(default)]
    pub one_other: Option<String>,

    /// other + one
    #[serde(default)]
    pub other_one: Option<String>,

    /// other + other (fallback, required)
    pub other_other: String,
    // Additional combinations can be added as needed
    // zero_zero, zero_one, etc.
}

impl DualPluralForms {
    /// Get the translation for two plural categories.
    #[must_use]
    pub fn get(&self, cat1: PluralCategory, cat2: PluralCategory) -> &str {
        match (cat1, cat2) {
            (PluralCategory::One, PluralCategory::One) => {
                self.one_one.as_deref().unwrap_or(&self.other_other)
            }
            (PluralCategory::One, _) => self.one_other.as_deref().unwrap_or(&self.other_other),
            (_, PluralCategory::One) => self.other_one.as_deref().unwrap_or(&self.other_other),
            _ => &self.other_other,
        }
    }
}

/// Translation value in TOML file.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum TranslationValue {
    /// Simple string translation
    Simple(String),
    /// Plural forms for single plural source
    Plural(PluralForms),
    /// Dual plural forms for two plural sources
    DualPlural(DualPluralForms),
}

/// Parsed translation file.
#[derive(Debug, Clone, Default)]
pub struct TranslationFile {
    /// Map of keys to translation values.
    translations: BTreeMap<String, TranslationValue>,
}

impl TranslationFile {
    /// Create a new empty translation file.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse a translation file from TOML content.
    ///
    /// # Errors
    ///
    /// Returns an error if the TOML is invalid or contains invalid translation values.
    pub fn parse(content: &str) -> Result<Self, toml::de::Error> {
        let translations: BTreeMap<String, TranslationValue> = toml::from_str(content)?;
        Ok(Self { translations })
    }

    /// Get a translation by key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&TranslationValue> {
        self.translations.get(key)
    }

    /// Check if a key exists.
    #[must_use]
    pub fn contains_key(&self, key: &str) -> bool {
        self.translations.contains_key(key)
    }

    /// Iterate over all translations.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &TranslationValue)> {
        self.translations.iter()
    }

    /// Number of translations.
    #[must_use]
    pub fn len(&self) -> usize {
        self.translations.len()
    }

    /// Check if empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.translations.is_empty()
    }
}

/// Validation error for translation files.
#[derive(Debug, Clone)]
pub enum ValidationError {
    /// Invalid plural form for locale.
    InvalidPluralForm {
        /// The translation key.
        key: String,
        /// The invalid plural category.
        form: PluralCategory,
        /// The locale code.
        locale: String,
    },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPluralForm { key, form, locale } => {
                write!(
                    f,
                    "Invalid plural form '{form:?}' for key '{key}' in locale '{locale}'"
                )
            }
        }
    }
}

impl core::error::Error for ValidationError {}
