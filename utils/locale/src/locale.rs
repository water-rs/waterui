//! Locale types and constants using ICU4X.

use core::ops::Deref;
use core::str::FromStr;
use std::collections::BTreeSet;

use icu_locid::{LanguageIdentifier, Locale as IcuLocale};
use icu_locid_transform::LocaleFallbacker;
use icu_provider::DataLocale;
use nami::impl_constant;
use waterui_core::Environment;
use waterui_core::extract::Extractor;

use crate::regional;

/// A locale wrapper around ICU4X `Locale`.
///
/// `Locale` (not `LanguageIdentifier`) preserves Unicode extension data like
/// `u-ca`, `u-hc`, and `u-nu`, enabling full locale preferences.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Locale(pub IcuLocale);

impl_constant!(Locale);

impl Locale {
    /// Create a new locale from an ICU locale.
    #[must_use]
    pub const fn new(id: IcuLocale) -> Self {
        Self(id)
    }

    /// Get the language identifier portion of this locale.
    #[must_use]
    pub fn id(&self) -> &LanguageIdentifier {
        &self.0.id
    }

    /// Return a canonicalized locale string.
    #[must_use]
    pub fn canonical_tag(&self) -> String {
        self.0.to_string()
    }
}

/// Keep legacy ergonomics: `locale.language`, `locale.region`, ...
/// by dereferencing to the language identifier subset.
impl Deref for Locale {
    type Target = LanguageIdentifier;

    fn deref(&self) -> &Self::Target {
        &self.0.id
    }
}

impl From<IcuLocale> for Locale {
    fn from(id: IcuLocale) -> Self {
        Self(id)
    }
}

impl From<Locale> for IcuLocale {
    fn from(locale: Locale) -> Self {
        locale.0
    }
}

impl From<LanguageIdentifier> for Locale {
    fn from(id: LanguageIdentifier) -> Self {
        Self(id.into())
    }
}

impl From<Locale> for LanguageIdentifier {
    fn from(locale: Locale) -> Self {
        locale.0.into()
    }
}

impl<'a> From<&'a Locale> for &'a LanguageIdentifier {
    fn from(locale: &'a Locale) -> Self {
        &locale.0.id
    }
}

impl FromStr for Locale {
    type Err = icu_locid::ParserError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        IcuLocale::from_str(s).map(Self)
    }
}

/// Built-in locale constants using ICU4X.
pub mod locales {
    use icu_locid::locale;

    use super::Locale;

    // =====================
    // Chinese variants
    // =====================

    /// Chinese (Simplified, China)
    pub const ZH_CN: Locale = Locale(locale!("zh-CN"));

    /// Chinese (Traditional, Taiwan)
    pub const ZH_TW: Locale = Locale(locale!("zh-TW"));

    /// Chinese (Traditional, Hong Kong)
    pub const ZH_HK: Locale = Locale(locale!("zh-HK"));

    /// Chinese (Simplified script)
    pub const ZH_HANS: Locale = Locale(locale!("zh-Hans"));

    /// Chinese (Traditional script)
    pub const ZH_HANT: Locale = Locale(locale!("zh-Hant"));

    // =====================
    // English variants
    // =====================

    /// English
    pub const EN: Locale = Locale(locale!("en"));

    /// English (United States)
    pub const EN_US: Locale = Locale(locale!("en-US"));

    /// English (United Kingdom)
    pub const EN_GB: Locale = Locale(locale!("en-GB"));

    // =====================
    // Portuguese variants
    // =====================

    /// Portuguese (Brazil)
    pub const PT_BR: Locale = Locale(locale!("pt-BR"));

    /// Portuguese (Portugal)
    pub const PT_PT: Locale = Locale(locale!("pt-PT"));

    // =====================
    // Serbian variants
    // =====================

    /// Serbian (Latin script)
    pub const SR_LATN: Locale = Locale(locale!("sr-Latn"));

    /// Serbian (Cyrillic script)
    pub const SR_CYRL: Locale = Locale(locale!("sr-Cyrl"));

    // =====================
    // Other languages
    // =====================

    /// Japanese
    pub const JA: Locale = Locale(locale!("ja"));

    /// Korean
    pub const KO: Locale = Locale(locale!("ko"));

    /// French
    pub const FR: Locale = Locale(locale!("fr"));

    /// German
    pub const DE: Locale = Locale(locale!("de"));

    /// Spanish
    pub const ES: Locale = Locale(locale!("es"));

    /// Russian
    pub const RU: Locale = Locale(locale!("ru"));

    /// Arabic
    pub const AR: Locale = Locale(locale!("ar"));

    /// Hindi
    pub const HI: Locale = Locale(locale!("hi"));

    /// Portuguese generic
    pub const PT: Locale = Locale(locale!("pt"));
}

/// Get the fallback chain for a locale using ICU4X.
///
/// This handles script-aware fallback:
/// - zh-TW → zh-Hant → zh (not zh-Hans!)
/// - sr-Latn → sr (stays separate from sr-Cyrl)
pub fn get_fallback_chain(locale: &Locale) -> Vec<Locale> {
    let mut seen = BTreeSet::new();
    let mut results = Vec::new();

    append_fallback_chain(locale, &mut seen, &mut results);

    let runtime_settings = regional::current_settings();
    if runtime_settings.locale_tag() == locale.canonical_tag() {
        for preferred in runtime_settings.preferred_languages() {
            if let Ok(preferred_locale) = Locale::from_str(preferred) {
                append_fallback_chain(&preferred_locale, &mut seen, &mut results);
            }
        }
    }

    results
}

fn append_fallback_chain(locale: &Locale, seen: &mut BTreeSet<String>, out: &mut Vec<Locale>) {
    let fallbacker = LocaleFallbacker::new();
    let config = fallbacker.for_config(Default::default());
    let mut iterator = config.fallback_for(DataLocale::from(locale.0.clone()));

    // Collect up to 10 fallback locales.
    for _ in 0..10 {
        let current = iterator.get();
        if current.is_und() {
            break;
        }
        let fallback = Locale(current.clone().into_locale());
        if seen.insert(fallback.canonical_tag()) {
            out.push(fallback);
        }
        iterator.step();
    }
}

impl Extractor for Locale {
    fn extract(env: &Environment) -> Result<Self, anyhow::Error> {
        if let Some(locale) = env.get::<Locale>().cloned() {
            return Ok(locale);
        }

        if let Some(context) = env.get::<regional::RegionalContext>() {
            return Ok(context.locale().clone());
        }

        Ok(regional::current_settings().locale().clone())
    }
}
