//! CLDR plural rules using ICU4X.

use icu_plurals::PluralRules;
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
/// ```rust
/// use waterui_locale::{PluralCategory, locales, select_plural};
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
    let rules = PluralRules::try_new_cardinal(locale.0.clone().into()).unwrap_or_else(|_| {
        // Fallback to English rules if locale not supported
        PluralRules::try_new_cardinal(icu_locale::locale!("en").into())
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

    float_value
        .abs()
        .to_string()
        .parse::<fixed_decimal::Decimal>()
        .map_or_else(
            |_| rules.category_for(0_u8),
            |decimal| rules.category_for(&decimal),
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
/// ```rust
/// use waterui_locale::{PluralCategory, locales, valid_categories};
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
    let rules = PluralRules::try_new_cardinal(locale.0.clone().into()).unwrap_or_else(|_| {
        PluralRules::try_new_cardinal(icu_locale::locale!("en").into())
            .expect("English plural rules should always be available")
    });

    rules.categories().collect()
}
