use alloc::string::ToString;
use time::Date;
use waterui_core::Str;
use waterui_core::{extract::Extractor, Error};

pub trait Formatter<T> {
    fn format(&self, value: &T) -> Str;
}

#[derive(Debug)]
pub struct DateFormatter {
    locale: Locale,
}

impl DateFormatter {
    pub fn get_locale(&self) -> &Locale {
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
            .ok_or(Error::msg("Locale not found"))?
            .clone();
        Ok(Self { locale })
    }
}

#[derive(Debug, Clone)]
pub struct Locale(pub Str);

impl Extractor for Locale {
    fn extract(env: &waterui_core::Environment) -> Result<Self, waterui_core::Error> {
        env.get::<Self>()
            .ok_or(waterui_core::Error::msg("Cannot determine locale"))
            .cloned()
    }
}
