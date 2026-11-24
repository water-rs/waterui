//! TOML translation file parser types re-exported from `waterui-locale-parser`.

use alloc::string::String;
use core::fmt;

extern crate alloc;

pub use waterui_locale_parser::{
    DualPluralForms, PluralForms, TranslationFile, TranslationParseError, TranslationValue,
};

use crate::plural::PluralCategory;

/// Validation error for translation files.
#[derive(Debug, Clone)]
#[non_exhaustive]
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
