//! Locale-aware number formatting.

use fixed_decimal::FixedDecimal;
use icu_decimal::FixedDecimalFormatter;

use crate::locale::Locale;

/// Format a number with locale-appropriate separators.
///
/// # Examples
///
/// ```rust,ignore
/// use waterui_locale::{format_number, locales};
///
/// // English: 1,234.56
/// assert_eq!(format_number(&locales::EN, 1234.56), "1,234.56");
///
/// // German: 1.234,56
/// assert_eq!(format_number(&locales::DE, 1234.56), "1.234,56");
///
/// // French: 1 234,56
/// assert_eq!(format_number(&locales::FR, 1234.56), "1 234,56");
/// ```
pub fn format_number(locale: &Locale, n: f64) -> String {
    let data_locale: icu_provider::DataLocale = locale.0.clone().into();
    let formatter = FixedDecimalFormatter::try_new(&data_locale, Default::default())
        .unwrap_or_else(|_| {
            let en_locale: icu_provider::DataLocale = icu_locid::langid!("en").into();
            FixedDecimalFormatter::try_new(&en_locale, Default::default())
                .expect("English number formatter should always be available")
        });

    // Convert f64 to FixedDecimal with reasonable precision
    // Handle both integers and floats appropriately
    let decimal = if n.fract() == 0.0 {
        FixedDecimal::from(n as i64)
    } else {
        // For floating point, format with up to 2 decimal places
        let scaled = (n * 100.0).round() as i64;
        let mut dec = FixedDecimal::from(scaled);
        dec.multiply_pow10(-2);
        dec
    };

    formatter.format(&decimal).to_string()
}

/// Format a number as a percentage.
///
/// # Examples
///
/// ```rust,ignore
/// use waterui_locale::{format_percent, locales};
///
/// // 0.75 → "75%"
/// assert_eq!(format_percent(&locales::EN, 0.75), "75%");
/// ```
pub fn format_percent(locale: &Locale, n: f64) -> String {
    // Convert to percentage value
    let percent_value = n * 100.0;
    format!("{}%", format_number(locale, percent_value))
}

/// Format a currency amount (display only, no conversion).
///
/// # Examples
///
/// ```rust,ignore
/// use waterui_locale::{format_currency, Currency, locales};
///
/// // US locale: $1,234.56
/// assert_eq!(format_currency(&locales::EN_US, 1234.56, Currency::USD), "$1,234.56");
///
/// // German locale: 1.234,56 €
/// assert_eq!(format_currency(&locales::DE, 1234.56, Currency::EUR), "1.234,56 €");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Currency {
    /// US Dollar
    USD,
    /// Euro
    EUR,
    /// British Pound
    GBP,
    /// Japanese Yen
    JPY,
    /// Chinese Yuan
    CNY,
}

impl Currency {
    /// Get the currency symbol.
    #[must_use]
    pub const fn symbol(&self) -> &'static str {
        match self {
            Self::USD => "$",
            Self::EUR => "€",
            Self::GBP => "£",
            Self::JPY => "¥",
            Self::CNY => "¥",
        }
    }
}

/// Format a currency amount with locale-appropriate formatting.
///
/// Note: This only handles display formatting, not currency conversion.
/// Exchange rates are real-time and outside the scope of i18n.
pub fn format_currency(locale: &Locale, amount: f64, currency: Currency) -> String {
    let formatted_amount = format_number(locale, amount);
    let symbol = currency.symbol();

    // Symbol position varies by locale
    // This is a simplified version - ICU4X has more sophisticated currency formatting
    match locale.language.as_str() {
        "de" | "fr" | "es" | "it" => format!("{formatted_amount} {symbol}"),
        _ => format!("{symbol}{formatted_amount}"),
    }
}
