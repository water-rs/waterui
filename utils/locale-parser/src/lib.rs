//! TOML translation file parser shared by `waterui-locale` and `waterui-macros`.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use toml::Value;
use toml::de::Error as TomlError;
use toml::value::Table;

const VALID_PLURAL_FIELDS: &[&str] = &["zero", "one", "two", "few", "many", "other"];

/// CLDR plural category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum PluralCategory {
    /// Zero items (used by Arabic, Latvian, Welsh, and others).
    Zero,
    /// Singular form.
    One,
    /// Dual form (used by Arabic, Slovenian, Welsh, and others).
    Two,
    /// Paucal / few form (used by Russian, Polish, Czech, and others).
    Few,
    /// Many form (used by Russian, Polish, Arabic, and others).
    Many,
    /// Default fallback form required by every language.
    Other,
}

impl PluralCategory {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "zero" => Some(Self::Zero),
            "one" => Some(Self::One),
            "two" => Some(Self::Two),
            "few" => Some(Self::Few),
            "many" => Some(Self::Many),
            "other" => Some(Self::Other),
            _ => None,
        }
    }

    fn parse_dual(name: &str) -> Option<(Self, Self)> {
        let (first, second) = name.split_once('_')?;
        Some((Self::parse(first)?, Self::parse(second)?))
    }
}

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluralForms {
    /// Form for zero items (some languages).
    pub zero: Option<String>,

    /// Form for one item.
    pub one: Option<String>,

    /// Form for two items (some languages).
    pub two: Option<String>,

    /// Form for few items (some languages).
    pub few: Option<String>,

    /// Form for many items (some languages).
    pub many: Option<String>,

    /// Fallback form (required and non-empty).
    pub other: String,
}

impl PluralForms {
    /// Returns the translation for a plural category, falling back to `other`.
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

    /// Iterates over all plural categories and their configured templates.
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
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DualPluralForms {
    /// Non-`(Other, Other)` category pairs.
    forms: BTreeMap<(PluralCategory, PluralCategory), String>,

    /// other + other (fallback, required and non-empty)
    other_other: String,
}

impl DualPluralForms {
    /// Returns the explicit template configured for `(cat1, cat2)`, if any.
    #[must_use]
    pub fn form(&self, cat1: PluralCategory, cat2: PluralCategory) -> Option<&str> {
        if cat1 == PluralCategory::Other && cat2 == PluralCategory::Other {
            Some(self.other_other.as_str())
        } else {
            self.forms.get(&(cat1, cat2)).map(String::as_str)
        }
    }

    /// Iterates over every configured category pair and its template in deterministic order.
    pub fn iter_forms(&self) -> impl Iterator<Item = ((PluralCategory, PluralCategory), &str)> {
        self.forms
            .iter()
            .map(|(&cats, text)| (cats, text.as_str()))
            .chain(core::iter::once((
                (PluralCategory::Other, PluralCategory::Other),
                self.other_other.as_str(),
            )))
    }

    /// one + one
    #[must_use]
    pub fn one_one(&self) -> Option<&str> {
        self.form(PluralCategory::One, PluralCategory::One)
    }

    /// one + other
    #[must_use]
    pub fn one_other(&self) -> Option<&str> {
        self.form(PluralCategory::One, PluralCategory::Other)
    }

    /// other + one
    #[must_use]
    pub fn other_one(&self) -> Option<&str> {
        self.form(PluralCategory::Other, PluralCategory::One)
    }

    /// other + other (fallback, required)
    #[must_use]
    pub const fn other_other(&self) -> &str {
        self.other_other.as_str()
    }

    /// Returns the translation for two plural categories, falling back through
    /// `(cat1, other)` and `(other, cat2)` to `other_other`.
    #[must_use]
    pub fn get(&self, cat1: PluralCategory, cat2: PluralCategory) -> &str {
        if let Some(exact) = self.form(cat1, cat2) {
            return exact;
        }
        if cat2 != PluralCategory::Other
            && let Some(first_specific) = self.form(cat1, PluralCategory::Other)
        {
            return first_specific;
        }
        if cat1 != PluralCategory::Other
            && let Some(second_specific) = self.form(PluralCategory::Other, cat2)
        {
            return second_specific;
        }
        &self.other_other
    }
}

/// Translation value in a TOML translation file.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TranslationValue {
    /// Simple string translation.
    Simple(String),
    /// Plural forms for a single plural source.
    Plural(PluralForms),
    /// Dual plural forms for two plural sources.
    DualPlural(DualPluralForms),
}

/// Error returned when a TOML translation file is malformed or violates the schema.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TranslationParseError {
    /// The input is not valid TOML.
    Syntax {
        /// Optional source identifier (such as a file path).
        source: Option<String>,
        /// Underlying TOML parser error message.
        message: String,
    },
    /// A top-level translation value is neither a string nor a plural table.
    InvalidValueType {
        /// Optional source identifier.
        source: Option<String>,
        /// The translation key whose value had an unsupported TOML type.
        key: String,
    },
    /// A single-plural table contains an unrecognized field name.
    UnknownPluralField {
        /// Optional source identifier.
        source: Option<String>,
        /// The translation key containing the unknown field.
        key: String,
        /// The unrecognized field name.
        field: String,
    },
    /// A dual-plural table contains an unrecognized field name.
    UnknownDualPluralField {
        /// Optional source identifier.
        source: Option<String>,
        /// The translation key containing the unknown field.
        key: String,
        /// The unrecognized field name.
        field: String,
    },
    /// A plural category field is not a TOML string.
    InvalidPluralFieldType {
        /// Optional source identifier.
        source: Option<String>,
        /// The translation key containing the invalid field.
        key: String,
        /// The field whose value was not a string.
        field: String,
    },
    /// A single-plural table omits `other` or sets it to an empty/whitespace string.
    MissingPluralOther {
        /// Optional source identifier.
        source: Option<String>,
        /// The translation key missing a non-empty `other` form.
        key: String,
    },
    /// A dual-plural table omits `other_other` or sets it to an empty/whitespace string.
    MissingDualPluralOther {
        /// Optional source identifier.
        source: Option<String>,
        /// The translation key missing a non-empty `other_other` form.
        key: String,
    },
}

fn write_source(f: &mut fmt::Formatter<'_>, source: Option<&str>) -> fmt::Result {
    if let Some(source) = source {
        write!(f, " in '{source}'")?;
    }
    Ok(())
}

impl fmt::Display for TranslationParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax { source, message } => {
                if let Some(source) = source {
                    write!(f, "Failed to parse '{source}': {message}")
                } else {
                    write!(f, "Failed to parse translation TOML: {message}")
                }
            }
            Self::InvalidValueType { source, key } => {
                write!(f, "Invalid translation value type for key '{key}'")?;
                write_source(f, source.as_deref())
            }
            Self::UnknownPluralField { source, key, field } => {
                write!(f, "Unknown plural field '{field}' for key '{key}'")?;
                write_source(f, source.as_deref())
            }
            Self::UnknownDualPluralField { source, key, field } => {
                write!(f, "Unknown dual plural field '{field}' for key '{key}'")?;
                write_source(f, source.as_deref())
            }
            Self::InvalidPluralFieldType { source, key, field } => {
                write!(f, "Plural field '{field}' for key '{key}'")?;
                write_source(f, source.as_deref())?;
                write!(f, " must be a string")
            }
            Self::MissingPluralOther { source, key } => {
                write!(f, "Plural key '{key}'")?;
                write_source(f, source.as_deref())?;
                write!(f, " must define non-empty 'other'")
            }
            Self::MissingDualPluralOther { source, key } => {
                write!(f, "Dual plural key '{key}'")?;
                write_source(f, source.as_deref())?;
                write!(f, " must define non-empty 'other_other'")
            }
        }
    }
}

impl Error for TranslationParseError {}

/// Parsed translation file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranslationFile {
    translations: BTreeMap<String, TranslationValue>,
}

impl TranslationFile {
    /// Creates a new empty translation file.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses a translation file from TOML content.
    ///
    /// # Errors
    ///
    /// Returns [`TranslationParseError`] if the TOML syntax or schema is invalid.
    pub fn parse(content: &str) -> Result<Self, TranslationParseError> {
        Self::parse_inner(content, None)
    }

    /// Parses a translation file from TOML content, attaching `source` to error messages.
    ///
    /// # Errors
    ///
    /// Returns [`TranslationParseError`] if the TOML syntax or schema is invalid.
    pub fn parse_named(content: &str, source: &str) -> Result<Self, TranslationParseError> {
        Self::parse_inner(content, Some(source))
    }

    fn parse_inner(content: &str, source: Option<&str>) -> Result<Self, TranslationParseError> {
        let table: Table =
            content
                .parse()
                .map_err(|err: TomlError| TranslationParseError::Syntax {
                    source: source.map(ToOwned::to_owned),
                    message: err.to_string(),
                })?;
        let mut translations = BTreeMap::new();

        for (key, value) in table {
            let parsed = match value {
                Value::String(text) => TranslationValue::Simple(text),
                Value::Table(subtable) => parse_plural_table(&subtable, &key, source)?,
                _ => {
                    return Err(TranslationParseError::InvalidValueType {
                        source: source.map(ToOwned::to_owned),
                        key,
                    });
                }
            };
            translations.insert(key, parsed);
        }

        Ok(Self { translations })
    }

    /// Returns a translation by key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&TranslationValue> {
        self.translations.get(key)
    }

    /// Checks whether a key exists.
    #[must_use]
    pub fn contains_key(&self, key: &str) -> bool {
        self.translations.contains_key(key)
    }

    /// Iterates over all translations in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &TranslationValue)> {
        self.translations.iter()
    }

    /// Consumes this file and returns its underlying map of entries.
    #[must_use]
    pub fn into_entries(self) -> BTreeMap<String, TranslationValue> {
        self.translations
    }

    /// Number of translations in this file.
    #[must_use]
    pub fn len(&self) -> usize {
        self.translations.len()
    }

    /// Returns `true` when this file contains no translations.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.translations.is_empty()
    }
}

fn parse_plural_field(
    table: &Table,
    field: &str,
    translation_key: &str,
    source: Option<&str>,
) -> Result<Option<String>, TranslationParseError> {
    match table.get(field) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(TranslationParseError::InvalidPluralFieldType {
            source: source.map(ToOwned::to_owned),
            key: translation_key.to_owned(),
            field: field.to_owned(),
        }),
    }
}

fn parse_plural_table(
    table: &Table,
    key: &str,
    source: Option<&str>,
) -> Result<TranslationValue, TranslationParseError> {
    let is_dual_plural = table.keys().any(|field| field.contains('_'));
    if is_dual_plural {
        let mut forms = BTreeMap::new();
        let mut other_other: Option<String> = None;
        for (field, value) in table {
            let Some((first, second)) = PluralCategory::parse_dual(field) else {
                return Err(TranslationParseError::UnknownDualPluralField {
                    source: source.map(ToOwned::to_owned),
                    key: key.to_owned(),
                    field: field.clone(),
                });
            };
            let Value::String(text) = value else {
                return Err(TranslationParseError::InvalidPluralFieldType {
                    source: source.map(ToOwned::to_owned),
                    key: key.to_owned(),
                    field: field.clone(),
                });
            };
            if first == PluralCategory::Other && second == PluralCategory::Other {
                other_other = Some(text.clone());
            } else {
                forms.insert((first, second), text.clone());
            }
        }

        let other_other = other_other
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| TranslationParseError::MissingDualPluralOther {
                source: source.map(ToOwned::to_owned),
                key: key.to_owned(),
            })?;

        Ok(TranslationValue::DualPlural(DualPluralForms {
            forms,
            other_other,
        }))
    } else {
        for field in table.keys() {
            if !VALID_PLURAL_FIELDS.contains(&field.as_str()) {
                return Err(TranslationParseError::UnknownPluralField {
                    source: source.map(ToOwned::to_owned),
                    key: key.to_owned(),
                    field: field.clone(),
                });
            }
        }

        let zero = parse_plural_field(table, "zero", key, source)?;
        let one = parse_plural_field(table, "one", key, source)?;
        let two = parse_plural_field(table, "two", key, source)?;
        let few = parse_plural_field(table, "few", key, source)?;
        let many = parse_plural_field(table, "many", key, source)?;
        let other = parse_plural_field(table, "other", key, source)?
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| TranslationParseError::MissingPluralOther {
                source: source.map(ToOwned::to_owned),
                key: key.to_owned(),
            })?;

        Ok(TranslationValue::Plural(PluralForms {
            zero,
            one,
            two,
            few,
            many,
            other,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_plural_and_dual_plural_entries() {
        let content = r#"
"Hello" = "Hello!"
"I have {#count} apple" = { one = "1 apple", other = "{count} apples" }
"{#a} and {#b}" = { one_one = "1 and 1", one_other = "1 and {b}", other_one = "{a} and 1", other_other = "{a} and {b}" }
"#;
        let file = TranslationFile::parse(content).expect("valid translation file should parse");
        assert_eq!(file.len(), 3);
        assert_eq!(
            file.get("Hello"),
            Some(&TranslationValue::Simple("Hello!".to_owned()))
        );
        match file.get("I have {#count} apple") {
            Some(TranslationValue::Plural(forms)) => {
                assert_eq!(forms.get(PluralCategory::One), "1 apple");
                assert_eq!(forms.get(PluralCategory::Other), "{count} apples");
                assert_eq!(forms.get(PluralCategory::Few), "{count} apples");
            }
            other => panic!("expected Plural, got {other:?}"),
        }
        match file.get("{#a} and {#b}") {
            Some(TranslationValue::DualPlural(forms)) => {
                assert_eq!(
                    forms.get(PluralCategory::One, PluralCategory::One),
                    "1 and 1"
                );
                assert_eq!(
                    forms.get(PluralCategory::One, PluralCategory::Other),
                    "1 and {b}"
                );
                assert_eq!(
                    forms.get(PluralCategory::Other, PluralCategory::One),
                    "{a} and 1"
                );
                assert_eq!(
                    forms.get(PluralCategory::Other, PluralCategory::Other),
                    "{a} and {b}"
                );
            }
            other => panic!("expected DualPlural, got {other:?}"),
        }
    }

    #[test]
    fn rejects_missing_or_empty_plural_other() {
        let missing = TranslationFile::parse(r#"apples = { one = "1 apple" }"#)
            .expect_err("missing 'other' must fail");
        assert!(matches!(
            missing,
            TranslationParseError::MissingPluralOther { .. }
        ));

        let empty = TranslationFile::parse(r#"apples = { one = "1 apple", other = "" }"#)
            .expect_err("empty 'other' must fail");
        assert!(matches!(
            empty,
            TranslationParseError::MissingPluralOther { .. }
        ));

        let blank = TranslationFile::parse(r#"apples = { one = "1 apple", other = "   " }"#)
            .expect_err("whitespace-only 'other' must fail");
        assert!(matches!(
            blank,
            TranslationParseError::MissingPluralOther { .. }
        ));
    }

    #[test]
    fn rejects_missing_or_empty_dual_plural_other_other() {
        let missing = TranslationFile::parse(r#"items = { one_one = "1 and 1" }"#)
            .expect_err("missing 'other_other' must fail");
        assert!(matches!(
            missing,
            TranslationParseError::MissingDualPluralOther { .. }
        ));

        let empty = TranslationFile::parse(r#"items = { one_one = "1 and 1", other_other = " " }"#)
            .expect_err("blank 'other_other' must fail");
        assert!(matches!(
            empty,
            TranslationParseError::MissingDualPluralOther { .. }
        ));
    }

    #[test]
    fn rejects_unknown_fields_and_invalid_types() {
        let unknown_single = TranslationFile::parse_named(
            r#"apples = { one = "1", manyy = "oops", other = "many" }"#,
            "i18n/en.toml",
        )
        .expect_err("unknown plural field must fail");
        assert!(unknown_single.to_string().contains("Unknown plural field"));
        assert!(unknown_single.to_string().contains("i18n/en.toml"));

        let unknown_dual = TranslationFile::parse(
            r#"items = { one_one = "1", few_manyy = "oops", other_other = "many" }"#,
        )
        .expect_err("unknown dual plural field must fail");
        assert!(matches!(
            unknown_dual,
            TranslationParseError::UnknownDualPluralField { .. }
        ));

        let extra_underscore_dual =
            TranslationFile::parse(r#"items = { one_one_one = "oops", other_other = "many" }"#)
                .expect_err("multi-underscore dual plural field must fail");
        assert!(matches!(
            extra_underscore_dual,
            TranslationParseError::UnknownDualPluralField { .. }
        ));

        let non_string_dual_field =
            TranslationFile::parse(r#"items = { few_few = 1, other_other = "many" }"#)
                .expect_err("non-string dual plural field must fail");
        assert!(matches!(
            non_string_dual_field,
            TranslationParseError::InvalidPluralFieldType { .. }
        ));

        let non_string_field = TranslationFile::parse(r#"apples = { one = 1, other = "many" }"#)
            .expect_err("non-string plural field must fail");
        assert!(matches!(
            non_string_field,
            TranslationParseError::InvalidPluralFieldType { .. }
        ));

        let non_string_value =
            TranslationFile::parse("count = 42").expect_err("integer value must fail");
        assert!(matches!(
            non_string_value,
            TranslationParseError::InvalidValueType { .. }
        ));
    }

    #[test]
    fn parses_all_cldr_dual_plural_categories_and_fallbacks() {
        let content = r#"
"{#apples} jabłko i {#oranges} pomarańcza" = {
  zero_two = "zero-two",
  one_one = "one-one",
  one_few = "one-few",
  one_other = "one-other",
  few_one = "few-one",
  few_few = "few-few",
  many_many = "many-many",
  other_few = "other-few",
  other_other = "other-other"
}
"#;
        let file = TranslationFile::parse_named(content, "i18n/pl.toml")
            .expect("CLDR dual plural categories should parse");
        let Some(TranslationValue::DualPlural(forms)) =
            file.get("{#apples} jabłko i {#oranges} pomarańcza")
        else {
            panic!("expected DualPlural");
        };
        assert_eq!(
            forms.get(PluralCategory::Zero, PluralCategory::Two),
            "zero-two"
        );
        assert_eq!(
            forms.get(PluralCategory::One, PluralCategory::Few),
            "one-few"
        );
        assert_eq!(
            forms.get(PluralCategory::Few, PluralCategory::Few),
            "few-few"
        );
        assert_eq!(
            forms.get(PluralCategory::Many, PluralCategory::Many),
            "many-many"
        );
        // Falls back via (cat1, other) before (other, cat2) and other_other.
        assert_eq!(
            forms.get(PluralCategory::One, PluralCategory::Many),
            "one-other"
        );
        assert_eq!(
            forms.get(PluralCategory::Many, PluralCategory::Few),
            "other-few"
        );
        assert_eq!(
            forms.get(PluralCategory::Many, PluralCategory::One),
            "other-other"
        );
    }
}
