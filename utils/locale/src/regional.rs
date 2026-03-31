//! Shared locale runtime for WaterUI backed by waterkit-regional.

use core::str::FromStr;
use std::error::Error;
use std::fmt;

use waterkit_regional::SystemSettingsContext;
use waterui_core::Environment;
use waterui_core::extract::Extractor;

use crate::locale::{Locale, locales};

pub use waterkit_regional::ListenerHandle;

/// Runtime locale context shared by reactive localized text rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionalContext {
    settings: SystemSettingsContext,
    locale: Locale,
}

impl RegionalContext {
    fn from_settings(settings: SystemSettingsContext) -> Self {
        let locale = Locale::from_str(settings.locale_tag()).unwrap_or(locales::EN_US);
        Self { settings, locale }
    }

    /// Returns the canonical BCP 47 locale tag.
    #[must_use]
    pub fn locale_tag(&self) -> &str {
        self.settings.locale_tag()
    }

    /// Returns the parsed locale value.
    #[must_use]
    pub fn locale(&self) -> &Locale {
        &self.locale
    }

    /// Returns preferred languages in descending priority.
    #[must_use]
    pub fn preferred_languages(&self) -> &[String] {
        self.settings.preferred_languages()
    }

    /// Returns the inferred region subtag, if present.
    #[must_use]
    pub fn region(&self) -> Option<&str> {
        self.settings.region()
    }

    /// Returns the IANA timezone identifier.
    #[must_use]
    pub fn timezone(&self) -> &str {
        self.settings.timezone()
    }

    /// Returns a copy with locale replaced while preserving other regional settings.
    #[must_use]
    pub fn with_locale(&self, locale: Locale) -> Self {
        let settings = self.settings.with_locale_tag(locale.canonical_tag());
        Self { settings, locale }
    }
}

/// Returned when a locale tag cannot be parsed as valid BCP 47.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidLocaleTag {
    tag: String,
}

impl InvalidLocaleTag {
    /// Returns the invalid locale tag payload.
    #[must_use]
    pub fn tag(&self) -> &str {
        &self.tag
    }
}

impl fmt::Display for InvalidLocaleTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid locale tag '{}'", self.tag)
    }
}

impl Error for InvalidLocaleTag {}

/// Returns the current shared runtime locale settings.
#[must_use]
pub fn current_settings() -> RegionalContext {
    RegionalContext::from_settings(waterkit_regional::current_settings())
}

/// Sets the shared runtime locale using a BCP 47 tag.
///
/// Returns an error when `tag` is not a valid locale identifier.
pub fn set_locale_tag(tag: impl AsRef<str>) -> Result<(), InvalidLocaleTag> {
    let locale_tag = tag.as_ref();
    let locale = Locale::from_str(locale_tag).map_err(|_| InvalidLocaleTag {
        tag: locale_tag.to_string(),
    })?;
    let _ = waterkit_regional::set_locale_tag(locale.canonical_tag());
    Ok(())
}

/// Registers a listener that will be invoked whenever the runtime locale changes.
///
/// The returned handle must be kept alive; dropping it unsubscribes the listener.
#[must_use]
pub fn register_listener(
    listener: impl Fn(&RegionalContext) + Send + Sync + 'static,
) -> ListenerHandle {
    waterkit_regional::register_listener(move |settings| {
        let context = RegionalContext::from_settings(settings);
        listener(&context);
    })
}

/// Initializes automatic locale refresh from host defaults.
pub fn start_auto_refresh_default() {
    waterkit_regional::start_auto_refresh_default();
}

impl Extractor for RegionalContext {
    fn extract(env: &Environment) -> Result<Self, anyhow::Error> {
        if let Some(context) = env.get::<RegionalContext>().cloned() {
            return Ok(context);
        }

        if let Some(locale) = env.get::<Locale>().cloned() {
            return Ok(current_settings().with_locale(locale));
        }

        Ok(current_settings())
    }
}
