//! The user's language and region preferences.

use objc2::MainThreadMarker;
use objc2_foundation::{NSCalendar, NSLocale, NSString};

use crate::notification::{self, NotificationName, NotificationObserver};

/// The user's first day of the week as a CLDR `fw` keyword value.
///
/// `"sun"` through `"sat"` when it differs from the day the locale
/// `language_tag` names starts weeks on; `None` when the user kept that
/// default. The preference lives in the user's current calendar, not in the
/// preferred-language tags, so a locale built from those tags alone loses
/// it.
///
/// # Panics
///
/// Never in practice: the frameworks report weekdays 1 through 7.
#[must_use]
pub fn first_weekday_preference(language_tag: &str) -> Option<&'static str> {
    const WEEKDAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
    let preferred = NSCalendar::currentCalendar().firstWeekday();
    let calendar = crate::date::gregorian();
    calendar.setLocale(Some(&NSLocale::localeWithLocaleIdentifier(
        &NSString::from_str(language_tag),
    )));
    (preferred != calendar.firstWeekday()).then(|| {
        *WEEKDAYS
            .get(preferred.wrapping_sub(1))
            .expect("NSCalendar reports weekdays 1 through 7")
    })
}

/// The user's preferred languages as BCP 47 tags, most preferred first.
///
/// The list reflects the settings at the moment of the call; observe
/// [`observe_changes`] to learn when it changes.
#[must_use]
pub fn preferred_languages() -> Vec<String> {
    NSLocale::preferredLanguages()
        .iter()
        .map(|tag| tag.to_string())
        .collect()
}

/// Calls `handler` on the main thread whenever the user's locale settings
/// change, until the returned guard is dropped.
pub fn observe_changes(
    mtm: MainThreadMarker,
    handler: impl Fn() + 'static,
) -> NotificationObserver {
    notification::observe(mtm, &NotificationName::current_locale_did_change(), handler)
}
