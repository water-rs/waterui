#![doc=include_str!("../README.md")]
#![no_std]
#![forbid(unsafe_code)]

#[cfg(feature = "std")]
mod std_on;
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;
use alloc::collections::BTreeMap;

use waterui_core::{Environment, SignalExt, Str, extract::Extractor, plugin::Plugin};
use waterui_text::{__nami::signal::IntoComputed, Text, TextConfig, locale::Locale};
/// Manages internationalization by storing and providing translations for different locales.
///
/// `I18n` acts as a central repository for key-value based translations. It is designed
/// to be integrated into the `WaterUI` `Environment` as a plugin. Once installed, it
/// automatically intercepts `Text` components and replaces their content with the
/// appropriate translation for the currently configured `Locale`.
///
/// # Examples
///
/// Basic setup and usage:
///
/// ```
/// use waterui_core::Environment;
/// use waterui_i18n::I18n;
/// use waterui_text::locale::Locale;
///
/// // Create a new I18n instance and add translations.
/// let mut i18n = I18n::new();
/// i18n.insert("en", "greeting", "Hello, World!");
/// i18n.insert("fr", "greeting", "Bonjour le monde!");
///
/// // Install the plugin into the environment.
/// let mut env = Environment::new();
/// i18n.install(&mut env);
///
/// // Set the current locale.
/// env.insert(Locale("fr".into()));
///
/// // Retrieve the underlying I18n instance and get a translation.
/// let i18n_from_env = env.get::<I18n>().unwrap();
/// assert_eq!(i18n_from_env.get("fr", "greeting"), "Bonjour le monde!");
/// ```
#[derive(Debug, Default, Clone)]
pub struct I18n {
    map: BTreeMap<Str, BTreeMap<Str, Str>>,
}

impl I18n {
    /// Creates a new, empty `I18n` instance.
    ///
    /// # Returns
    ///
    /// A `const` `I18n` struct ready to have translations inserted.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            map: BTreeMap::new(),
        }
    }

    /// Inserts a translation for a given locale and key.
    ///
    /// If the locale does not exist, it will be created. If the key already exists
    /// for the given locale, its value will be overwritten.
    ///
    /// # Parameters
    ///
    /// - `locale`: The locale identifier (e.g., "en", "fr-CA").
    /// - `key`: The translation key (e.g., "greeting").
    /// - `value`: The translated string.
    pub fn insert(&mut self, locale: impl Into<Str>, key: impl Into<Str>, value: impl Into<Str>) {
        self.map
            .entry(locale.into())
            .or_default()
            .insert(key.into(), value.into());
    }

    /// Retrieves a translation for a given locale and key.
    ///
    /// If the translation is not found for the specified locale, this method will
    /// return the `key` itself as the fallback value. This is useful for development,
    /// as missing translations will be clearly visible in the UI.
    ///
    /// # Parameters
    ///
    /// - `locale`: The locale to search in.
    /// - `key`: The translation key.
    ///
    /// # Returns
    ///
    /// The translated `Str`, or the `key` if no translation is found.
    pub fn get(&self, locale: &str, key: impl Into<Str>) -> Str {
        let key = key.into();
        self.try_get(locale, &key).cloned().unwrap_or(key)
    }

    /// Tries to retrieve a translation for a given locale and key, returning an `Option`.
    ///
    /// This method is useful when you need to check for the existence of a translation
    /// without receiving a fallback value.
    ///
    /// # Parameters
    ///
    /// - `locale`: The locale to search in.
    /// - `key`: The translation key.
    ///
    /// # Returns
    ///
    /// - `Some(&Str)` if the translation is found.
    /// - `None` if the locale or key does not exist.
    #[must_use]
    pub fn try_get(&self, locale: &str, key: &str) -> Option<&Str> {
        self.map.get(locale).and_then(|map| map.get(key))
    }
}

impl Plugin for I18n {
    /// Installs the `I18n` instance into the `WaterUI` `Environment`.
    ///
    /// This method performs two key actions:
    /// 1. Inserts the `I18n` instance itself into the environment, making it accessible
    ///    via `env.get::<I18n>()`.
    /// 2. Inserts a `TextConfig` hook that intercepts `Text` views. The hook uses the
    ///    current `Locale` from the environment to look up and apply translations.
    fn install(self, env: &mut Environment) {
        env.insert(self);
        env.insert_hook(|env, mut config: TextConfig| {
            // Extract the current locale from the environment.
            let Locale(locale) = Locale::extract(env).unwrap_or_default();
            let env = env.clone();

            // Create a computed signal that re-evaluates whenever the text content changes.
            config.content = config
                .content
                .map(move |content| {
                    // Look up the translation.
                    if let Some(i18n) = env.get::<Self>() {
                        i18n.get(&locale, content)
                    } else {
                        // If the I18n plugin isn't found, return the original content.
                        content
                    }
                })
                .into_computed();

            Text::from(config)
        });
    }
}
