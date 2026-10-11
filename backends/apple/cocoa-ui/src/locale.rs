//! The user's language and region preferences.

use objc2::MainThreadMarker;
use objc2_foundation::NSLocale;

use crate::notification::{self, NotificationName, NotificationObserver};

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
