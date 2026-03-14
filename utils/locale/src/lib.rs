//! # `WaterUI` i18n
//!
//! Internationalization (i18n) support for `WaterUI` applications.
//!
//! ## Features
//!
//! - **Type-safe locales**: Using ICU4X for locale identifiers
//! - **CLDR plural rules**: Proper pluralization for all languages
//! - **Smart fallback**: zh-TW → zh-Hant → zh (not zh-Hans!)
//! - **Unit wrappers**: `Length<Meter>`, `Temperature<Celsius>` with automatic conversion
//! - **`LocalizedDisplay`**: Trait for locale-aware formatting
//!
//! ## Usage
//!
//! ```rust,ignore
//! use waterui_locale::{text, locales};
//!
//! let count = 5;
//! let greeting = text!("I have {#count} apple");
//! // With en: "I have 5 apples"
//! // With zh: "我有5个苹果"
//! ```

#![forbid(unsafe_code)]

pub mod catalog;
pub mod format;
pub mod locale;
pub mod parser;
pub mod plural;
pub mod regional;
mod system;

// Re-exports
pub use catalog::TranslationCatalog;
pub use format::unit::{Feet, Kilometer, Length, LengthUnit, Meter, Mile};
pub use format::{LocalizedDisplay, LocalizedList};
pub use locale::{Locale, locales};
pub use plural::{PluralCategory, select_plural};

/// Returns a reactive binding for the effective locale in the given environment.
#[must_use]
pub fn locale_binding(env: &waterui_core::Environment) -> nami::Binding<Locale> {
    if let Some(context) = env.get::<regional::RegionalContext>().cloned() {
        return nami::Binding::custom(nami::Container::new(context.locale().clone()));
    }

    if let Some(locale) = env.get::<Locale>().cloned() {
        return nami::Binding::custom(nami::Container::new(locale));
    }

    system::runtime_locale_binding()
}

#[cfg(test)]
mod tests {
    use super::*;
    use format::date::{DateStyle, SimpleDate, format_date};
    use format::unit::{Gram, Kilogram, Mass};

    // =========================================================
    // Plural Rules Tests
    // =========================================================

    #[test]
    fn test_plural_selection_english() {
        // English: 1 → One, 0/2+ → Other
        assert_eq!(select_plural(&locales::EN, 1), PluralCategory::One);
        assert_eq!(select_plural(&locales::EN, 0), PluralCategory::Other);
        assert_eq!(select_plural(&locales::EN, 2), PluralCategory::Other);
        assert_eq!(select_plural(&locales::EN, 100), PluralCategory::Other);
    }

    #[test]
    fn test_plural_selection_english_fractional() {
        // English fractional values should be "other"
        assert_eq!(select_plural(&locales::EN, 1.2), PluralCategory::Other);
        assert_eq!(select_plural(&locales::EN, 0.5), PluralCategory::Other);
    }

    #[test]
    fn test_plural_selection_chinese() {
        // Chinese has no plural distinction - all → Other
        assert_eq!(select_plural(&locales::ZH_CN, 0), PluralCategory::Other);
        assert_eq!(select_plural(&locales::ZH_CN, 1), PluralCategory::Other);
        assert_eq!(select_plural(&locales::ZH_CN, 100), PluralCategory::Other);
    }

    #[test]
    fn test_plural_selection_japanese() {
        // Japanese has no plural distinction - all → Other
        assert_eq!(select_plural(&locales::JA, 0), PluralCategory::Other);
        assert_eq!(select_plural(&locales::JA, 1), PluralCategory::Other);
        assert_eq!(select_plural(&locales::JA, 100), PluralCategory::Other);
    }

    #[test]
    fn test_plural_selection_korean() {
        // Korean has no plural distinction - all → Other
        assert_eq!(select_plural(&locales::KO, 0), PluralCategory::Other);
        assert_eq!(select_plural(&locales::KO, 1), PluralCategory::Other);
        assert_eq!(select_plural(&locales::KO, 100), PluralCategory::Other);
    }

    #[test]
    fn test_plural_selection_german() {
        // German: 1 → One, others → Other
        assert_eq!(select_plural(&locales::DE, 1), PluralCategory::One);
        assert_eq!(select_plural(&locales::DE, 0), PluralCategory::Other);
        assert_eq!(select_plural(&locales::DE, 2), PluralCategory::Other);
    }

    #[test]
    fn test_plural_selection_french() {
        // French: 0,1 → One, 2+ → Other (differs from English!)
        assert_eq!(select_plural(&locales::FR, 0), PluralCategory::One);
        assert_eq!(select_plural(&locales::FR, 1), PluralCategory::One);
        assert_eq!(select_plural(&locales::FR, 2), PluralCategory::Other);
    }

    #[test]
    fn test_plural_selection_spanish() {
        // Spanish: 1 → One, others → Other
        assert_eq!(select_plural(&locales::ES, 1), PluralCategory::One);
        assert_eq!(select_plural(&locales::ES, 0), PluralCategory::Other);
        assert_eq!(select_plural(&locales::ES, 2), PluralCategory::Other);
    }

    #[test]
    fn test_plural_selection_russian() {
        // Russian: complex rules with One, Few, Many
        assert_eq!(select_plural(&locales::RU, 1), PluralCategory::One);
        assert_eq!(select_plural(&locales::RU, 2), PluralCategory::Few);
        assert_eq!(select_plural(&locales::RU, 5), PluralCategory::Many);
        assert_eq!(select_plural(&locales::RU, 21), PluralCategory::One);
        assert_eq!(select_plural(&locales::RU, 22), PluralCategory::Few);
    }

    #[test]
    fn test_plural_selection_negative_uses_absolute_value() {
        // CLDR plural rules use absolute value.
        assert_eq!(select_plural(&locales::EN, -1), PluralCategory::One);
        assert_eq!(select_plural(&locales::EN, -2), PluralCategory::Other);
    }

    // =========================================================
    // Length Unit Tests
    // =========================================================

    #[test]
    fn test_length_conversion() {
        let km = Length::<Kilometer>::new(1.0);
        let m = km.to::<Meter>();
        assert!((m.value() - 1000.0).abs() < 0.001);

        let mile = Length::<Mile>::new(1.0);
        let m2 = mile.to::<Meter>();
        assert!((m2.value() - 1609.344).abs() < 0.001);
    }

    #[test]
    fn test_length_addition() {
        let km = Length::<Kilometer>::new(1.0);
        let m = Length::<Meter>::new(500.0);
        let total = km + m;
        assert!((total.value() - 1.5).abs() < 0.001);
    }

    #[test]
    fn test_localized_display_length() {
        let distance = Length::<Meter>::new(100.0);

        // English
        let en_str = distance.to_localized_string(&locales::EN);
        assert!(en_str.contains("100"));
        assert!(en_str.contains("m"));

        // Chinese
        let zh_str = distance.to_localized_string(&locales::ZH_CN);
        assert!(zh_str.contains("100"));
        assert!(zh_str.contains("米"));

        // Japanese
        let ja_str = distance.to_localized_string(&locales::JA);
        assert!(ja_str.contains("100"));
        assert!(ja_str.contains("メートル"));

        // Korean
        let ko_str = distance.to_localized_string(&locales::KO);
        assert!(ko_str.contains("100"));
        assert!(ko_str.contains("미터"));
    }

    // =========================================================
    // Mass Unit Tests
    // =========================================================

    #[test]
    fn test_mass_localized_display() {
        let mass = Mass::<Kilogram>::new(5.0);

        // Chinese
        let zh_str = mass.to_localized_string(&locales::ZH_CN);
        assert!(zh_str.contains("5"));
        assert!(zh_str.contains("公斤"));

        // Japanese
        let ja_str = mass.to_localized_string(&locales::JA);
        assert!(ja_str.contains("5"));
        assert!(ja_str.contains("キログラム"));

        // Korean
        let ko_str = mass.to_localized_string(&locales::KO);
        assert!(ko_str.contains("5"));
        assert!(ko_str.contains("킬로그램"));

        // European languages use "kg"
        let de_str = mass.to_localized_string(&locales::DE);
        assert!(de_str.contains("5"));
        assert!(de_str.contains("kg"));
    }

    #[test]
    fn test_mass_conversion() {
        let kg = Mass::<Kilogram>::new(1.0);
        let g = kg.to::<Gram>();
        assert!((g.value() - 1000.0).abs() < 0.001);
    }

    // =========================================================
    // Date Formatting Tests (using Go's canonical date: 2006.01.02)
    // =========================================================

    #[test]
    fn test_date_formatting_english() {
        let date = SimpleDate::new(2006, 3, 20);
        let short = format_date(&locales::EN, &date, DateStyle::Short);
        let long = format_date(&locales::EN, &date, DateStyle::Long);
        assert!(short.contains('/'));
        assert!(short.contains("20"));
        assert!(long.contains("2006"));
        assert!(long.to_lowercase().contains("mar"));
    }

    #[test]
    fn test_date_formatting_german() {
        let date = SimpleDate::new(2006, 3, 20);
        let short = format_date(&locales::DE, &date, DateStyle::Short);
        let long = format_date(&locales::DE, &date, DateStyle::Long);
        assert!(short.contains('.'));
        assert!(short.contains("20"));
        assert!(long.contains("2006"));
        assert!(long.contains("Mär") || long.to_lowercase().contains("mar"));
    }

    #[test]
    fn test_date_formatting_french() {
        let date = SimpleDate::new(2006, 3, 20);
        let short = format_date(&locales::FR, &date, DateStyle::Short);
        let long = format_date(&locales::FR, &date, DateStyle::Long);
        assert!(short.contains("2006"));
        assert!(short.contains('/'));
        assert!(long.contains("2006"));
        assert!(long.to_lowercase().contains("mar"));
    }

    #[test]
    fn test_date_formatting_spanish() {
        let date = SimpleDate::new(2006, 3, 20);
        let short = format_date(&locales::ES, &date, DateStyle::Short);
        let long = format_date(&locales::ES, &date, DateStyle::Long);
        assert!(short.contains('/'));
        assert!(short.contains("20"));
        assert!(long.contains("2006"));
        assert!(long.to_lowercase().contains("mar"));
    }

    #[test]
    fn test_date_formatting_japanese() {
        let date = SimpleDate::new(2006, 3, 20);
        let short = format_date(&locales::JA, &date, DateStyle::Short);
        let long = format_date(&locales::JA, &date, DateStyle::Long);
        assert!(short.contains("2006"));
        assert!(long.contains('年'));
        assert!(long.contains('月'));
        assert!(long.contains('日'));
    }

    #[test]
    fn test_date_formatting_korean() {
        let date = SimpleDate::new(2006, 3, 20);
        let short = format_date(&locales::KO, &date, DateStyle::Short);
        let long = format_date(&locales::KO, &date, DateStyle::Long);
        assert!(short.contains("20"));
        assert!(long.contains('년'));
        assert!(long.contains('월'));
        assert!(long.contains('일'));
    }

    #[test]
    fn test_date_formatting_chinese() {
        let date = SimpleDate::new(2006, 3, 20);
        let short = format_date(&locales::ZH_CN, &date, DateStyle::Short);
        let long = format_date(&locales::ZH_CN, &date, DateStyle::Long);
        assert!(short.contains("2006"));
        assert!(long.contains('年'));
        assert!(long.contains('月'));
        assert!(long.contains('日'));
    }

    // =========================================================
    // Translation File Parsing Tests
    // =========================================================

    #[test]
    fn test_translation_file_parsing() {
        use parser::{TranslationFile, TranslationValue};

        let toml_content = r#"
"Hello, World!" = "Hello, World!"
"Goodbye" = "Farewell"
"I have {#count} apple" = { one = "I have {count} apple", other = "I have {count} apples" }
"#;

        let file = TranslationFile::parse(toml_content).unwrap();
        assert_eq!(file.len(), 3);

        // Check simple translation
        match file.get("Goodbye") {
            Some(TranslationValue::Simple(s)) => assert_eq!(s, "Farewell"),
            _ => panic!("Expected simple translation"),
        }

        // Check plural translation
        match file.get("I have {#count} apple") {
            Some(TranslationValue::Plural(forms)) => {
                assert_eq!(forms.get(PluralCategory::One), "I have {count} apple");
                assert_eq!(forms.get(PluralCategory::Other), "I have {count} apples");
            }
            _ => panic!("Expected plural translation"),
        }
    }

    #[test]
    fn test_valid_categories() {
        let en_cats = plural::valid_categories(&locales::EN);
        assert!(en_cats.contains(&PluralCategory::One));
        assert!(en_cats.contains(&PluralCategory::Other));

        let zh_cats = plural::valid_categories(&locales::ZH_CN);
        // Chinese only has "other"
        assert!(zh_cats.contains(&PluralCategory::Other));
        assert!(!zh_cats.contains(&PluralCategory::One));
    }
}
