//! Locale types and constants using ICU4X.

use core::ops::Deref;
use core::str::FromStr;

use icu_locid::LanguageIdentifier;
use icu_locid_transform::LocaleFallbacker;
use waterui_core::extract::Extractor;
use waterui_core::Environment;

/// A locale identifier wrapper around ICU4X `LanguageIdentifier`.
///
/// This wrapper allows us to implement traits like `Extractor` for locale types.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Locale(pub LanguageIdentifier);

impl Locale {
    /// Create a new locale from a language identifier.
    #[must_use]
    pub const fn new(id: LanguageIdentifier) -> Self {
        Self(id)
    }

    /// Get the underlying language identifier.
    #[must_use]
    pub const fn id(&self) -> &LanguageIdentifier {
        &self.0
    }
}

impl Deref for Locale {
    type Target = LanguageIdentifier;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<LanguageIdentifier> for Locale {
    fn from(id: LanguageIdentifier) -> Self {
        Self(id)
    }
}

impl From<Locale> for LanguageIdentifier {
    fn from(locale: Locale) -> Self {
        locale.0
    }
}

impl<'a> From<&'a Locale> for &'a LanguageIdentifier {
    fn from(locale: &'a Locale) -> Self {
        &locale.0
    }
}

impl FromStr for Locale {
    type Err = icu_locid::ParserError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        LanguageIdentifier::from_str(s).map(Self)
    }
}

/// Built-in locale constants using ICU4X.
pub mod locales {
    use icu_locid::langid;

    use super::Locale;

    // =====================
    // Chinese variants
    // =====================

    /// Chinese (Simplified, China)
    pub const ZH_CN: Locale = Locale(langid!("zh-CN"));

    /// Chinese (Traditional, Taiwan)
    pub const ZH_TW: Locale = Locale(langid!("zh-TW"));

    /// Chinese (Traditional, Hong Kong)
    pub const ZH_HK: Locale = Locale(langid!("zh-HK"));

    /// Chinese (Simplified script)
    pub const ZH_HANS: Locale = Locale(langid!("zh-Hans"));

    /// Chinese (Traditional script)
    pub const ZH_HANT: Locale = Locale(langid!("zh-Hant"));

    // =====================
    // English variants
    // =====================

    /// English
    pub const EN: Locale = Locale(langid!("en"));

    /// English (United States)
    pub const EN_US: Locale = Locale(langid!("en-US"));

    /// English (United Kingdom)
    pub const EN_GB: Locale = Locale(langid!("en-GB"));

    // =====================
    // Portuguese variants
    // =====================

    /// Portuguese (Brazil)
    pub const PT_BR: Locale = Locale(langid!("pt-BR"));

    /// Portuguese (Portugal)
    pub const PT_PT: Locale = Locale(langid!("pt-PT"));

    // =====================
    // Serbian variants
    // =====================

    /// Serbian (Latin script)
    pub const SR_LATN: Locale = Locale(langid!("sr-Latn"));

    /// Serbian (Cyrillic script)
    pub const SR_CYRL: Locale = Locale(langid!("sr-Cyrl"));

    // =====================
    // Other languages
    // =====================

    /// Japanese
    pub const JA: Locale = Locale(langid!("ja"));

    /// Korean
    pub const KO: Locale = Locale(langid!("ko"));

    /// French
    pub const FR: Locale = Locale(langid!("fr"));

    /// German
    pub const DE: Locale = Locale(langid!("de"));

    /// Spanish
    pub const ES: Locale = Locale(langid!("es"));

    /// Russian
    pub const RU: Locale = Locale(langid!("ru"));

    /// Arabic
    pub const AR: Locale = Locale(langid!("ar"));
}

/// Get the fallback chain for a locale using ICU4X.
///
/// This handles script-aware fallback:
/// - zh-TW → zh-Hant → zh (not zh-Hans!)
/// - sr-Latn → sr (stays separate from sr-Cyrl)
pub fn get_fallback_chain(locale: &Locale) -> Vec<Locale> {
    let fallbacker = LocaleFallbacker::new();
    let config = fallbacker.for_config(Default::default());
    let id: LanguageIdentifier = locale.0.clone();
    let mut iterator = config.fallback_for(id.into());
    let mut results = Vec::new();

    // Collect up to 10 fallback locales
    for _ in 0..10 {
        let item = iterator.get();
        let loc = item.clone().into_locale();
        // Stop if we've reached the "und" (undefined) locale
        // Check by comparing language string to empty
        if loc.id.language.is_empty() && loc.id.script.is_none() && loc.id.region.is_none() {
            break;
        }
        results.push(Locale(loc.id));
        iterator.step();
    }

    results
}

impl Extractor for Locale {
    fn extract(env: &Environment) -> Result<Self, anyhow::Error> {
        env.get::<Locale>()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Locale not found in environment"))
    }
}
