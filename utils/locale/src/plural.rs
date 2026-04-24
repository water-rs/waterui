//! CLDR plural rules using ICU4X.

use core::str::FromStr;

use icu_plurals::PluralOperands;
use icu_plurals::PluralRules;
use icu_provider::DataLocale;
use num::ToPrimitive;

use crate::locale::Locale;

/// Re-export `PluralCategory` from ICU4X.
pub use icu_plurals::PluralCategory;

/// Select the plural category for a number using CLDR rules.
///
/// Uses ICU4X for accurate CLDR-compliant plural selection.
///
/// # Panics
///
/// Panics only if ICU4X cannot construct the built-in English plural rules
/// used as the fallback for unsupported locales.
///
/// # Examples
///
/// ```rust,ignore
/// use waterui_locale::{select_plural, locales, PluralCategory};
///
/// // English: 1 → One, 2+ → Other
/// assert_eq!(select_plural(&locales::EN, &1), PluralCategory::One);
/// assert_eq!(select_plural(&locales::EN, &2), PluralCategory::Other);
///
/// // Chinese: all → Other (no plural distinction)
/// assert_eq!(select_plural(&locales::ZH_CN, &1), PluralCategory::Other);
/// assert_eq!(select_plural(&locales::ZH_CN, &100), PluralCategory::Other);
///
/// // Russian: complex rules with One, Few, Many, Other
/// assert_eq!(select_plural(&locales::RU, &1), PluralCategory::One);
/// assert_eq!(select_plural(&locales::RU, &2), PluralCategory::Few);
/// assert_eq!(select_plural(&locales::RU, &5), PluralCategory::Many);
/// ```
pub fn select_plural<N: ToPrimitive + ?Sized>(locale: &Locale, n: &N) -> PluralCategory {
    // Create plural rules for the locale
    let data_locale: DataLocale = locale.0.clone().into();
    let rules = PluralRules::try_new_cardinal(&data_locale).unwrap_or_else(|_| {
        // Fallback to English rules if locale not supported
        let en_locale: DataLocale = icu_locid::langid!("en").into();
        PluralRules::try_new_cardinal(&en_locale)
            .expect("English plural rules should always be available")
    });

    // Preserve fractional operands when possible (e.g. "1.2" -> Other in English).
    let float_value = n.to_f64().unwrap_or(0.0);
    if float_value.is_finite() && float_value.fract() == 0.0 {
        if let Some(value) = n.to_i64() {
            return rules.category_for(value);
        }
        if let Some(value) = n.to_u64() {
            return rules.category_for(value);
        }
    }

    let operand_str = float_value.abs().to_string();
    PluralOperands::from_str(&operand_str).map_or_else(
        |_| rules.category_for(0_u8),
        |operands| rules.category_for(operands),
    )
}

/// Get all valid plural categories for a locale.
///
/// This is useful for validation - checking if a translation file
/// uses valid plural forms for the target locale.
///
/// # Panics
///
/// Panics only if ICU4X cannot construct the built-in English plural rules
/// used as the fallback for unsupported locales.
///
/// # Examples
///
/// ```rust,ignore
/// use waterui_locale::{valid_categories, locales};
///
/// // Chinese only has Other
/// let zh_cats = valid_categories(&locales::ZH_CN);
/// assert_eq!(zh_cats, vec![PluralCategory::Other]);
///
/// // English has One and Other
/// let en_cats = valid_categories(&locales::EN);
/// assert!(en_cats.contains(&PluralCategory::One));
/// assert!(en_cats.contains(&PluralCategory::Other));
/// ```
#[must_use]
pub fn valid_categories(locale: &Locale) -> Vec<PluralCategory> {
    let data_locale: DataLocale = locale.0.clone().into();
    let rules = PluralRules::try_new_cardinal(&data_locale).unwrap_or_else(|_| {
        let en_locale: DataLocale = icu_locid::langid!("en").into();
        PluralRules::try_new_cardinal(&en_locale)
            .expect("English plural rules should always be available")
    });

    rules.categories().collect()
}
