//! Locale-aware number formatting.

use std::sync::atomic::{AtomicBool, Ordering};

use fixed_decimal::Decimal;
use icu_decimal::{DecimalFormatter, options::DecimalFormatterOptions};
use icu_experimental::dimension::{
    currency::{CurrencyType, formatter::CurrencyFormatter, options::CurrencyFormatterOptions},
    percent::{formatter::PercentFormatter, options::PercentFormatterOptions},
};

use crate::locale::{Locale, locales};

static NUMBER_FORMATTER_FALLBACK_LOGGED: AtomicBool = AtomicBool::new(false);
static PERCENT_FORMATTER_FALLBACK_LOGGED: AtomicBool = AtomicBool::new(false);
static CURRENCY_FORMATTER_FALLBACK_LOGGED: AtomicBool = AtomicBool::new(false);

fn format_non_finite(n: f64) -> Option<String> {
    if n.is_nan() {
        return Some("NaN".to_string());
    }
    if n.is_infinite() {
        return Some(if n.is_sign_negative() {
            "-Infinity".to_string()
        } else {
            "Infinity".to_string()
        });
    }
    None
}

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

    // `Display` for floats prints the shortest decimal that round-trips to the
    // same `f64`, and never in scientific notation. Asking for a fixed count of
    // fractional digits instead surfaces the binary representation itself:
    // `format!("{:.15}", 1234.56)` is `1234.559999999999945`, and trimming
    // trailing zeros does not rescue it — which is precisely what a number
    // formatter must never show a reader.
    let buf = format!("{n}");

    if let Ok(parsed) = buf.parse() {
        return parsed;
    }

    format!("{n:.2}")
        .parse()
        .expect("a finite f64 printed to two fractional digits is a plain decimal literal, which `Decimal` always parses")
}

pub(crate) fn format_number_text(locale: &Locale, value: &str) -> String {
    // A numeric argument's `Display` text is a plain decimal, except for the
    // non-finite floats (`NaN`, `inf`, `-inf`), which have no decimal form: they
    // are shown as Rust prints them.
    let Ok(decimal) = value.parse::<Decimal>() else {
        return value.to_owned();
    };
    decimal_formatter(locale).map_or_else(
        || value.to_owned(),
        |formatter| formatter.format(&decimal).to_string(),
    )
}

/// Format a number with locale-appropriate separators.
///
/// # Examples
///
/// ```rust
/// use waterui_locale::{format_number, locales};
///
/// // English: 1,234.56
/// assert_eq!(format_number(&locales::EN, 1234.56), "1,234.56");
///
/// // German: 1.234,56
/// assert_eq!(format_number(&locales::DE, 1234.56), "1.234,56");
///
/// // French: 1 234,56 — the group separator is a narrow no-break space
/// assert_eq!(format_number(&locales::FR, 1234.56), "1\u{202f}234,56");
/// ```
#[must_use]
pub fn format_number(locale: &Locale, n: f64) -> String {
    if let Some(non_finite) = format_non_finite(n) {
        return non_finite;
    }

    let decimal = to_decimal(n);
    decimal_formatter(locale).map_or_else(
        || n.to_string(),
        |formatter| formatter.format(&decimal).to_string(),
    )
}

fn percent_formatter(locale: &Locale) -> Option<PercentFormatter<DecimalFormatter>> {
    match PercentFormatter::try_new((&locale.0).into(), PercentFormatterOptions::default()) {
        Ok(formatter) => Some(formatter),
        Err(primary_error) => match PercentFormatter::try_new(
            (&locales::EN.0).into(),
            PercentFormatterOptions::default(),
        ) {
            Ok(formatter) => Some(formatter),
            Err(fallback_error) => {
                if PERCENT_FORMATTER_FALLBACK_LOGGED
                    .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
                {
                    tracing::warn!(
                        locale = locale.canonical_tag(),
                        primary_error = %primary_error,
                        fallback_error = %fallback_error,
                        "waterui-locale percent formatter unavailable"
                    );
                }
                None
            }
        },
    }
}

/// Format a number as a percentage.
///
/// # Examples
///
/// ```rust
/// use waterui_locale::{format_percent, locales};
///
/// // 0.75 → "75%"
/// assert_eq!(format_percent(&locales::EN, 0.75), "75%");
/// ```
#[must_use]
pub fn format_percent(locale: &Locale, n: f64) -> String {
    if let Some(non_finite) = format_non_finite(n) {
        return non_finite;
    }

    let mut percent = to_decimal(n);
    percent.multiply_pow10(2);
    percent.trim_start();
    percent_formatter(locale).map_or_else(
        || format!("{percent}%"),
        |formatter| formatter.format(&percent).to_string(),
    )
}

/// A currency [`format_currency`] knows how to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
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
    /// South Korean Won
    KRW,
    /// Bahraini Dinar
    BHD,
    /// Kuwaiti Dinar
    KWD,
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
            Self::KRW => "₩",
            Self::BHD => "BD",
            Self::KWD => "KD",
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
            Self::KRW => "KRW",
            Self::BHD => "BHD",
            Self::KWD => "KWD",
        }
    }

    const fn icu_type(self) -> CurrencyType {
        const fn parse(code: &str) -> CurrencyType {
            match CurrencyType::try_from_str(code) {
                Ok(currency) => currency,
                Err(_) => panic!("Currency::code must be a 3-letter ASCII ISO 4217 code"),
            }
        }
        match self {
            Self::USD => const { parse("USD") },
            Self::EUR => const { parse("EUR") },
            Self::GBP => const { parse("GBP") },
            Self::JPY => const { parse("JPY") },
            Self::CNY => const { parse("CNY") },
            Self::KRW => const { parse("KRW") },
            Self::BHD => const { parse("BHD") },
            Self::KWD => const { parse("KWD") },
        }
    }
}

fn currency_formatter(
    locale: &Locale,
    currency: CurrencyType,
) -> Option<CurrencyFormatter<DecimalFormatter>> {
    match CurrencyFormatter::try_new_symbol_narrow(
        (&locale.0).into(),
        currency,
        CurrencyFormatterOptions::default(),
    ) {
        Ok(formatter) => Some(formatter),
        Err(primary_error) => match CurrencyFormatter::try_new_symbol_narrow(
            (&locales::EN.0).into(),
            currency,
            CurrencyFormatterOptions::default(),
        ) {
            Ok(formatter) => Some(formatter),
            Err(fallback_error) => {
                if CURRENCY_FORMATTER_FALLBACK_LOGGED
                    .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
                {
                    tracing::warn!(
                        locale = locale.canonical_tag(),
                        currency = %currency.iso_code(),
                        primary_error = %primary_error,
                        fallback_error = %fallback_error,
                        "waterui-locale currency formatter unavailable"
                    );
                }
                None
            }
        },
    }
}

/// Format a currency amount with locale-appropriate formatting.
///
/// Note: This handles symbol placement, currency fraction digits, and locale
/// number formatting from CLDR; no exchange-rate conversion is performed.
///
/// # Examples
///
/// ```rust
/// use waterui_locale::{Currency, format_currency, locales};
///
/// // US locale: $1,234.56
/// assert_eq!(format_currency(&locales::EN_US, 1234.56, Currency::USD), "$1,234.56");
///
/// // German locale: 1.234,56 €
/// assert_eq!(
///     format_currency(&locales::DE, 1234.56, Currency::EUR),
///     "1.234,56\u{a0}€"
/// );
/// ```
#[must_use]
pub fn format_currency(locale: &Locale, amount: f64, currency: Currency) -> String {
    if let Some(non_finite) = format_non_finite(amount) {
        return non_finite;
    }

    let decimal = to_decimal(amount);
    currency_formatter(locale, currency.icu_type()).map_or_else(
        || format!("{}{}", currency.symbol(), format_number(locale, amount)),
        |formatter| formatter.format_fixed_decimal(&decimal).to_string(),
    )
}
