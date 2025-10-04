use alloc::string::ToString;
use nami::impl_constant;
use time::Date;
use waterui_core::Str;
use waterui_core::{Error, extract::Extractor};

/// Trait for formatting values into locale-aware strings.
///
/// This trait allows different types to be formatted according to
/// locale-specific rules and conventions.
pub trait Formatter<T> {
    /// Formats the given value into a localized string representation.
    fn format(&self, value: &T) -> Str;
}

/// A formatter for dates that respects locale settings.
///
/// This formatter can convert date values into locale-appropriate
/// string representations.
#[derive(Debug)]
pub struct DateFormatter {
    locale: Locale,
}

impl DateFormatter {
    /// Returns a reference to the locale used by this formatter.
    #[must_use]
    pub const fn get_locale(&self) -> &Locale {
        &self.locale
    }
}

impl Formatter<Date> for DateFormatter {
    fn format(&self, value: &Date) -> Str {
        value.to_string().into()
    }
}

impl Extractor for DateFormatter {
    fn extract(env: &waterui_core::Environment) -> Result<Self, waterui_core::Error> {
        let locale = env
            .get::<Locale>()
            .ok_or_else(|| Error::msg("Locale not found"))?
            .clone();
        Ok(Self { locale })
    }
}

/// Represents a locale for internationalization.
///
/// This wraps a string identifier that represents a specific locale,
/// such as "en-US" or "fr-FR", used for locale-aware formatting.
#[derive(Debug, Clone)]
pub struct Locale(pub Str);

impl_constant!(Locale);

impl Default for Locale {
    fn default() -> Self {
        Self("en-US".into())
    }
}

impl Extractor for Locale {
    fn extract(env: &waterui_core::Environment) -> Result<Self, waterui_core::Error> {
        env.get::<Self>()
            .ok_or_else(|| waterui_core::Error::msg("Cannot determine locale"))
            .cloned()
    }
}
