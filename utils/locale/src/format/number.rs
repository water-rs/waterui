//! Locale-aware number formatting.

use std::sync::atomic::{AtomicBool, Ordering};

use fixed_decimal::Decimal;
use icu_decimal::{DecimalFormatter, options::DecimalFormatterOptions};

use crate::locale::{Locale, locales};

static NUMBER_FORMATTER_FALLBACK_LOGGED: AtomicBool = AtomicBool::new(false);

fn decimal_formatter(locale: &Locale) -> Option<DecimalFormatter> {
    match DecimalFormatter::try_new(locale.0.clone().into(), DecimalFormatterOptions::default()) {
        Ok(formatter) => Some(formatter),
        Err(primary_error) => match DecimalFormatter::try_new(
            locales::EN.0.into(),
            DecimalFormatterOptions::default(),
        ) {
            Ok(formatter) => Some(formatter),
            Err(fallback_error) => {
                if NUMBER_FORMATTER_FALLBACK_LOGGED
                    .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
                {
                    tracing::warn!(
                        locale = locale.canonical_tag(),
                        primary_error = %primary_error,
                        fallback_error = %fallback_error,
                        "waterui-locale number formatter unavailable"
                    );
                }
                None
            }
        },
    }
}

fn to_decimal(n: f64) -> Decimal {
    debug_assert!(n.is_finite(), "to_decimal expects a finite number");

    // Preserve up to 15 fractional digits while avoiding scientific notation.
    let mut buf = format!("{n:.15}");
    if buf.contains('.') {
        while buf.ends_with('0') {
            buf.pop();
        }
        if buf.ends_with('.') {
            buf.pop();
        }
    }

    if let Ok(parsed) = buf.parse() {
        return parsed;
    }

    format!("{n:.2}")
        .parse()
        .expect("fixed decimal fallback formatting should always produce a parseable decimal")
}

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
#[must_use]
pub fn format_number(locale: &Locale, n: f64) -> String {
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n.is_infinite() {
        return if n.is_sign_negative() {
            "-Infinity".to_string()
        } else {
            "Infinity".to_string()
        };
    }

    let decimal = to_decimal(n);
    decimal_formatter(locale).map_or_else(
        || n.to_string(),
        |formatter| formatter.format(&decimal).to_string(),
    )
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
#[must_use]
pub fn format_percent(locale: &Locale, n: f64) -> String {
    let percent_value = n * 100.0;
    let formatted = format_number(locale, percent_value);

    // Locale-aware percent symbol and spacing.
    match locale.language.as_str() {
        "fr" => format!("{formatted}\u{00A0}%"),
        "ar" => format!("{formatted}٪"),
        _ => format!("{formatted}%"),
    }
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
            Self::JPY | Self::CNY => "¥",
        }
    }

    /// Get ISO 4217 currency code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::USD => "USD",
            Self::EUR => "EUR",
            Self::GBP => "GBP",
            Self::JPY => "JPY",
            Self::CNY => "CNY",
        }
    }
}

/// Format a currency amount with locale-appropriate formatting.
///
/// Note: This handles symbol placement and locale number formatting;
/// no exchange-rate conversion is performed.
#[must_use]
pub fn format_currency(locale: &Locale, amount: f64, currency: Currency) -> String {
    let formatted_amount = format_number(locale, amount);
    let symbol = currency.symbol();

    // Symbol position varies by locale.
    match locale.language.as_str() {
        "de" | "fr" | "es" | "it" | "pt" | "ru" => {
            format!("{formatted_amount}\u{00A0}{symbol}")
        }
        "ar" => format!("{symbol}\u{00A0}{formatted_amount}"),
        _ => format!("{symbol}{formatted_amount}"),
    }
}
