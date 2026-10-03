//! The system locale in the environment.
//!
//! `Locale.getDefault().toLanguageTag()` is the platform's preferred
//! language — it becomes the app's `Locale` plus a `Binding<Locale>` that
//! locale-sensitive views resolve through; the shared regional runtime sees
//! the same tag. [`refresh`] rewrites both when a configuration change
//! arrives, so localized text follows the Settings change without a
//! relaunch.

use alloc::string::String;
use core::str::FromStr;

use waterui::reactive::{Binding, Container};
use waterui_backend_core::Environment;
use waterui_locale::{Locale, regional};

/// The locale `Locale.getDefault()` names, or English when the tag does not
/// parse.
fn system_locale(tag: &str) -> Locale {
    Locale::from_str(tag).unwrap_or_else(|error| {
        tracing::warn!("invalid locale tag {tag:?}: {error}; falling back to en");
        Locale::from_str("en").expect("`en` is a valid BCP 47 tag")
    })
}

/// Reads `Locale.getDefault()` — the platform's preferred language.
fn platform_tag(env: &mut jni::Env) -> String {
    crate::jvm::globals()
        .bindings()
        .locale_tag(env)
        .unwrap_or_else(|error| {
            tracing::warn!("Locale.getDefault() failed: {error}; falling back to en");
            String::from("en")
        })
}

/// Installs `locale` into `env`: the regional runtime's tag, the
/// `RegionalContext`, the locale value itself, and a `Binding<Locale>` every
/// later locale write goes through.
fn install_locale_value(env: &mut Environment, locale: Locale) {
    let binding = env.get::<Binding<Locale>>().cloned();
    regional::set_locale_tag(locale.canonical_tag())
        .expect("a parsed locale tag must remain valid when published");
    let context = regional::current_settings().with_locale(&locale);
    env.insert(context);
    env.insert(locale.clone());
    if let Some(binding) = binding {
        binding.set(locale);
    } else {
        env.insert(Binding::custom(Container::new(locale)));
    }
}

/// Installs the platform's current locale and returns the binding
/// [`refresh`] republishes through on configuration changes — the observer
/// the Apple backend hangs off `NSLocale.currentLocaleDidChange` is here
/// just a republish, driven by `onConfigurationChanged`.
pub(crate) fn install(env: &mut Environment) -> Binding<Locale> {
    crate::jvm::with_env(|jenv| install_locale_value(env, system_locale(&platform_tag(jenv))));
    env.get::<Binding<Locale>>()
        .cloned()
        .expect("install_locale_value installs the locale binding")
}

/// Re-reads `Locale.getDefault()` and republishes the binding — what an
/// `onConfigurationChanged` with a locale diff forwards.
pub(crate) fn refresh(binding: &Binding<Locale>) {
    crate::jvm::with_env(|env| {
        let locale = system_locale(&platform_tag(env));
        regional::set_locale_tag(locale.canonical_tag())
            .expect("a parsed locale tag must remain valid when published");
        binding.set(locale);
    });
}
