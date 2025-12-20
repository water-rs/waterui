//! CLDR plural rules using ICU4X.

use icu_plurals::PluralRules;
use icu_provider::DataLocale;
use num::ToPrimitive;

use crate::locale::Locale;

/// Re-export PluralCategory from ICU4X.
pub use icu_plurals::PluralCategory;

/// Select the plural category for a number using CLDR rules.
///
/// Uses ICU4X for accurate CLDR-compliant plural selection.
///
/// # Examples
///
/// ```rust,ignore
/// use waterui_locale::{select_plural, locales, PluralCategory};
///
/// // English: 1 → One, 2+ → Other
/// assert_eq!(select_plural(&locales::EN, 1), PluralCategory::One);
/// assert_eq!(select_plural(&locales::EN, 2), PluralCategory::Other);
///
/// // Chinese: all → Other (no plural distinction)
/// assert_eq!(select_plural(&locales::ZH_CN, 1), PluralCategory::Other);
/// assert_eq!(select_plural(&locales::ZH_CN, 100), PluralCategory::Other);
///
/// // Russian: complex rules with One, Few, Many, Other
/// assert_eq!(select_plural(&locales::RU, 1), PluralCategory::One);
/// assert_eq!(select_plural(&locales::RU, 2), PluralCategory::Few);
/// assert_eq!(select_plural(&locales::RU, 5), PluralCategory::Many);
/// ```
pub fn select_plural<N: ToPrimitive>(locale: &Locale, n: N) -> PluralCategory {
    let n_i64 = n.to_i64().unwrap_or(0);

    // Create plural rules for the locale
    let data_locale: DataLocale = locale.0.clone().into();
    let rules = PluralRules::try_new_cardinal(&data_locale).unwrap_or_else(|_| {
        // Fallback to English rules if locale not supported
        let en_locale: DataLocale = icu_locid::langid!("en").into();
        PluralRules::try_new_cardinal(&en_locale)
            .expect("English plural rules should always be available")
    });

    // Use integer operands for proper plural selection
    rules.category_for(n_i64 as usize)
}

/// Get all valid plural categories for a locale.
///
/// This is useful for validation - checking if a translation file
/// uses valid plural forms for the target locale.
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
pub fn valid_categories(locale: &Locale) -> Vec<PluralCategory> {
    let data_locale: DataLocale = locale.0.clone().into();
    let rules = PluralRules::try_new_cardinal(&data_locale).unwrap_or_else(|_| {
        let en_locale: DataLocale = icu_locid::langid!("en").into();
        PluralRules::try_new_cardinal(&en_locale)
            .expect("English plural rules should always be available")
    });

    rules.categories().collect()
}
