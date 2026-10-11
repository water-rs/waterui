//! The system locale in the environment.
//!
//! The platform's preferred language list becomes the app's `Locale` plus a
//! `Binding<Locale>` that locale-sensitive views resolve through; the shared
//! regional runtime sees the same tag. The returned observer rewrites both
//! when the system list changes, so localized text follows the Settings
//! change without a relaunch.

use alloc::string::String;
use core::str::FromStr;

use waterui::reactive::{Binding, Container};
use waterui_backend_core::Environment;
use waterui_locale::{Locale, regional};

/// Installs `locale` into `env`: the regional runtime's tag, the
/// `RegionalContext`, the locale value itself, and a `Binding<Locale>` every
/// later locale write goes through.
fn install_locale_value(env: &mut Environment, locale: Locale) {
    let binding = env.get::<Binding<Locale>>().cloned();
    regional::set_locale_tag(locale.canonical_tag())
        .expect("a parsed locale tag must remain valid when published");
    let context = regional::current_settings().with_locale(&locale);
    env.insert(locale.clone());
    env.insert(context);
    if let Some(binding) = binding {
        binding.set(locale);
    } else {
        env.insert(Binding::custom(Container::new(locale)));
    }
}

/// The locale the platform's preferred-language list names, or English when
/// the list is empty.
fn system_locale(languages: &[String]) -> Locale {
    let tag = languages.first().map_or("en", String::as_str);
    Locale::from_str(tag).unwrap_or_else(|error| {
        tracing::warn!("invalid preferred language {tag:?}: {error}; falling back to en");
        Locale::from_str("en").expect("`en` is a valid BCP 47 tag")
    })
}

/// Publishes a new system locale: the binding views resolve through, and the
/// regional runtime's tag. The environment's own snapshot is not rewritten —
/// it was read at install; everything that changes reads the binding.
fn publish_locale(binding: &Binding<Locale>, locale: Locale) {
    regional::set_locale_tag(locale.canonical_tag())
        .expect("a parsed locale tag must remain valid when published");
    binding.set(locale);
}

/// Installs the platform's current locale and returns a keep-alive for the
/// change observer. The observer borrows nothing, so the opaque return does
/// not capture `env`'s lifetime.
#[must_use]
pub fn install(env: &mut Environment, mtm: cocoa_ui::MainThreadMarker) -> impl Sized + use<> {
    let languages = cocoa_ui::locale::preferred_languages();
    install_locale_value(env, system_locale(&languages));
    let binding = env
        .get::<Binding<Locale>>()
        .cloned()
        .expect("install_locale_value installs the locale binding");
    cocoa_ui::locale::observe_changes(mtm, move || {
        publish_locale(
            &binding,
            system_locale(&cocoa_ui::locale::preferred_languages()),
        );
    })
}
