//! Percent and currency formatting must follow CLDR, not per-language tables.
//! Expected strings are CLDR patterns via ICU4X.

use waterui_locale::{Currency, format_currency, format_percent, locales};

#[test]
fn repro_49_german_percent_has_nbsp_before_sign() {
    // CLDR de percent pattern: `#,##0 %` with U+00A0.
    assert_eq!(format_percent(&locales::DE, 0.75), "75\u{a0}%");
}

#[test]
fn repro_49_japanese_yen_has_no_fraction_digits() {
    // CLDR JPY has 0 fraction digits.
    assert_eq!(
        format_currency(&locales::EN_US, 1234.56, Currency::JPY),
        "¥1,235"
    );
}

#[test]
fn cldr_percent_across_locales_including_cjk_and_arabic() {
    assert_eq!(format_percent(&locales::EN_US, 0.75), "75%");
    assert_eq!(format_percent(&locales::FR, 0.75), "75\u{a0}%");
    assert_eq!(format_percent(&locales::ZH_CN, 0.75), "75%");
    assert_eq!(format_percent(&locales::JA, 0.75), "75%");
    assert_eq!(format_percent(&locales::KO, 0.75), "75%");
    // 0.29 * 100.0 in f64 is 28.999999999999996; Decimal scaling keeps 29%.
    assert_eq!(format_percent(&locales::EN_US, 0.29), "29%");
    assert_eq!(format_percent(&locales::EN_US, 0.0), "0%");
    assert_eq!(format_percent(&locales::EN_US, 0.005), "0.5%");
    assert_eq!(format_percent(&locales::EN_US, -0.75), "-75%");
    assert_eq!(format_percent(&locales::DE, -0.75), "-75\u{a0}%");

    let ar_eg: waterui_locale::Locale = "ar-EG".parse().unwrap();
    let ar_percent = format_percent(&ar_eg, 0.75);
    assert_eq!(ar_percent, "٧٥\u{200e}%\u{200e}");
    let tr: waterui_locale::Locale = "tr".parse().unwrap();
    assert_eq!(format_percent(&tr, 0.75), "%75");
}

#[test]
fn cldr_currency_fraction_digits_and_symbol_placement() {
    assert_eq!(
        format_currency(&locales::EN_US, 1234.56, Currency::USD),
        "$1,234.56"
    );
    assert_eq!(
        format_currency(&locales::DE, 1234.56, Currency::EUR),
        "1.234,56\u{a0}€"
    );
    assert_eq!(
        format_currency(&locales::EN_US, 1234.56, Currency::KRW),
        "₩1,235"
    );
    assert_eq!(
        format_currency(&locales::KO, 1234.56, Currency::KRW),
        "₩1,235"
    );
    assert_eq!(
        format_currency(&locales::JA, 1234.56, Currency::JPY),
        "￥1,235"
    );
    assert_eq!(
        format_currency(&locales::ZH_CN, 1234.56, Currency::CNY),
        "¥1,234.56"
    );
    assert_eq!(
        format_currency(&locales::EN_US, 12.3, Currency::BHD),
        "BHD\u{a0}12.300"
    );
    assert_eq!(
        format_currency(&locales::EN_US, 12.3, Currency::KWD),
        "KWD\u{a0}12.300"
    );
}
