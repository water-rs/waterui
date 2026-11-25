//! Reactive `text!` interpolation tests for `jiff` date/time and relative-time formatters.

use jiff::{civil, tz::TimeZone};
use waterui::locale::{DateStyle, RelativeTime, TimeStyle, locales};
use waterui::prelude::*;

#[test]
fn text_macro_formats_jiff_date_signal_reactively_across_locales() {
    let locale = Binding::new(locales::EN);
    let mut env = Environment::new();
    env.insert(locale.clone());

    let picked = Binding::new(civil::date(2026, 3, 20));
    let view = text!(
        "Due {due}",
        due = picked.map(|date| date.formatted(DateStyle::Long))
    );
    let content = view.resolve(&env).content();

    assert_eq!(
        content.snapshot().to_plain(),
        "Due \u{2068}March 20, 2026\u{2069}"
    );

    // Changing the locale re-formats the same date without rebuilding the view.
    locale.set(locales::JA);
    assert_eq!(
        content.snapshot().to_plain(),
        "Due \u{2068}2026年3月20日\u{2069}"
    );

    locale.set(locales::ZH_CN);
    assert_eq!(
        content.snapshot().to_plain(),
        "Due \u{2068}2026年3月20日\u{2069}"
    );

    locale.set(locales::KO);
    assert_eq!(
        content.snapshot().to_plain(),
        "Due \u{2068}2026년 3월 20일\u{2069}"
    );

    // Changing the picked date re-formats under the active locale.
    picked.set(civil::date(2026, 12, 5));
    assert_eq!(
        content.snapshot().to_plain(),
        "Due \u{2068}2026년 12월 5일\u{2069}"
    );

    locale.set(locales::DE);
    assert_eq!(
        content.snapshot().to_plain(),
        "Due \u{2068}5. Dezember 2026\u{2069}"
    );
}

#[test]
fn text_macro_formats_constant_zoned_and_relative_time_reactively() {
    let locale = Binding::new(locales::EN);
    let mut env = Environment::new();
    env.insert(locale.clone());

    let zoned = civil::date(2026, 3, 20)
        .at(9, 30, 0, 0)
        .to_zoned(TimeZone::UTC)
        .expect("UTC conversion must succeed");
    let when = zoned.formatted((DateStyle::Long, TimeStyle::Long));
    let relative = RelativeTime::days(3);

    let zoned_content = text!("{when}").resolve(&env).content();
    let relative_content = text!("{relative}").resolve(&env).content();

    assert!(
        zoned_content
            .snapshot()
            .to_plain()
            .contains("March 20, 2026")
    );
    assert_eq!(
        relative_content.snapshot().to_plain(),
        "\u{2068}in 3 days\u{2069}"
    );

    locale.set(locales::ZH_CN);
    assert!(
        zoned_content
            .snapshot()
            .to_plain()
            .contains("2026年3月20日")
    );
    assert_eq!(
        relative_content.snapshot().to_plain(),
        "\u{2068}3天后\u{2069}"
    );

    locale.set(locales::JA);
    assert_eq!(
        relative_content.snapshot().to_plain(),
        "\u{2068}3 日後\u{2069}"
    );

    locale.set(locales::KO);
    assert_eq!(
        relative_content.snapshot().to_plain(),
        "\u{2068}3일 후\u{2069}"
    );
}
