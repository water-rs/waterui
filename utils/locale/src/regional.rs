//! Shared locale runtime for `WaterUI`.

use core::str::FromStr;
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use nami::Binding;
use waterui_core::Environment;
use waterui_core::extract::Extractor;

use crate::locale::{Locale, locales};

type Listener = Arc<dyn Fn(RegionalContext) + Send + Sync + 'static>;

struct Runtime {
    state: Mutex<State>,
    auto_refresh_started: AtomicBool,
}

struct State {
    current: RegionalContext,
    override_context: Option<RegionalContext>,
    listeners: HashMap<u64, Listener>,
    next_listener_id: u64,
}

impl Runtime {
    fn new() -> Self {
        Self {
            state: Mutex::new(State {
                current: detect_system_context(),
                override_context: None,
                listeners: HashMap::new(),
                next_listener_id: 1,
            }),
            auto_refresh_started: AtomicBool::new(false),
        }
    }

    fn remove_listener(&self, id: u64) {
        let removed = {
            let mut state = self
                .state
                .lock()
                .expect("waterui-locale regional runtime mutex poisoned");
            state.listeners.remove(&id)
        };
        drop(removed);
    }
}

fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(Runtime::new)
}

/// Runtime locale context shared by reactive localized text rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionalContext {
    locale_tag: String,
    preferred_languages: Vec<String>,
    region: Option<String>,
    timezone: String,
    locale: Locale,
}

impl RegionalContext {
    fn new(
        locale_tag: impl Into<String>,
        preferred_languages: Vec<String>,
        timezone: impl Into<String>,
    ) -> Self {
        let locale_tag = normalize_locale_tag(&locale_tag.into());

        let mut preferred_languages: Vec<String> = preferred_languages
            .into_iter()
            .map(|language| normalize_locale_tag(&language))
            .filter(|language| !language.is_empty())
            .collect();

        if preferred_languages.is_empty() {
            preferred_languages.push(locale_tag.clone());
        }

        if !preferred_languages
            .iter()
            .any(|language| language == &locale_tag)
        {
            preferred_languages.insert(0, locale_tag.clone());
        }

        let locale = Locale::from_str(&locale_tag).unwrap_or(locales::EN_US);
        let timezone = timezone.into();

        Self {
            region: extract_region(&locale_tag),
            locale_tag,
            preferred_languages,
            timezone: if timezone.is_empty() {
                "UTC".to_string()
            } else {
                timezone
            },
            locale,
        }
    }

    /// Returns the canonical BCP 47 locale tag.
    #[must_use]
    pub fn locale_tag(&self) -> &str {
        &self.locale_tag
    }

    /// Returns the parsed locale value.
    #[must_use]
    pub const fn locale(&self) -> &Locale {
        &self.locale
    }

    /// Returns preferred languages in descending priority.
    #[must_use]
    pub fn preferred_languages(&self) -> &[String] {
        &self.preferred_languages
    }

    /// Returns the inferred region subtag, if present.
    #[must_use]
    pub fn region(&self) -> Option<&str> {
        self.region.as_deref()
    }

    /// Returns the IANA timezone identifier.
    #[must_use]
    pub fn timezone(&self) -> &str {
        &self.timezone
    }

    /// Returns a copy with locale replaced while preserving other regional settings.
    #[must_use]
    pub fn with_locale(&self, locale: &Locale) -> Self {
        Self::new(
            locale.canonical_tag(),
            self.preferred_languages.clone(),
            self.timezone.clone(),
        )
    }
}

/// Subscription handle returned by [`register_listener`].
///
/// Dropping the handle unregisters the listener.
#[derive(Debug)]
pub struct ListenerHandle {
    id: Option<u64>,
}

impl ListenerHandle {
    /// Unregisters the listener immediately.
    pub fn unregister(mut self) {
        self.unregister_inner();
    }

    fn unregister_inner(&mut self) {
        if let Some(id) = self.id.take() {
            runtime().remove_listener(id);
        }
    }
}

impl Drop for ListenerHandle {
    fn drop(&mut self) {
        self.unregister_inner();
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
///
/// # Panics
///
/// Panics if the regional runtime mutex is poisoned.
#[must_use]
pub fn current_settings() -> RegionalContext {
    runtime()
        .state
        .lock()
        .expect("waterui-locale regional runtime mutex poisoned")
        .current
        .clone()
}

/// Sets the shared runtime locale using a BCP 47 tag.
///
/// Returns an error when `tag` is not a valid locale identifier.
///
/// # Errors
///
/// Returns [`InvalidLocaleTag`] when `tag` cannot be parsed as a valid BCP 47
/// locale identifier.
pub fn set_locale_tag(tag: impl AsRef<str>) -> Result<(), InvalidLocaleTag> {
    let locale_tag = tag.as_ref();
    let locale = Locale::from_str(locale_tag).map_err(|_| InvalidLocaleTag {
        tag: locale_tag.to_string(),
    })?;
    let _ = set_settings(current_settings().with_locale(&locale));
    Ok(())
}

/// Registers a listener that will be invoked whenever the runtime locale changes.
///
/// The returned handle must be kept alive; dropping it unsubscribes the listener.
///
/// # Panics
///
/// Panics if the regional runtime mutex is poisoned.
#[must_use]
pub fn register_listener(
    listener: impl Fn(&RegionalContext) + Send + Sync + 'static,
) -> ListenerHandle {
    let listener: Listener = Arc::new(move |context| listener(&context));

    let (id, current) = {
        let mut state = runtime()
            .state
            .lock()
            .expect("waterui-locale regional runtime mutex poisoned");
        let id = state.next_listener_id;
        state.next_listener_id = state
            .next_listener_id
            .checked_add(1)
            .expect("waterui-locale regional listener id overflow");
        state.listeners.insert(id, listener.clone());
        (id, state.current.clone())
    };

    listener(current);

    ListenerHandle { id: Some(id) }
}

/// Initializes automatic locale refresh from host defaults.
pub fn start_auto_refresh_default() {
    start_auto_refresh(Duration::from_secs(2));
}

fn start_auto_refresh(interval: Duration) {
    if runtime().auto_refresh_started.swap(true, Ordering::SeqCst) {
        return;
    }

    let interval = if interval.is_zero() {
        Duration::from_secs(2)
    } else {
        interval
    };

    let _handle = thread::Builder::new()
        .name("waterui-locale-regional-refresh".to_string())
        .spawn(move || {
            loop {
                thread::sleep(interval);
                let _ = refresh();
            }
        })
        .expect("failed to spawn waterui-locale regional refresh thread");
}

fn refresh() -> RegionalContext {
    let next = {
        let state = runtime()
            .state
            .lock()
            .expect("waterui-locale regional runtime mutex poisoned");
        state
            .override_context
            .clone()
            .unwrap_or_else(detect_system_context)
    };

    publish_if_changed(next)
}

fn set_settings(context: RegionalContext) -> RegionalContext {
    let listeners = {
        let mut state = runtime()
            .state
            .lock()
            .expect("waterui-locale regional runtime mutex poisoned");
        state.override_context = Some(context.clone());
        if state.current == context {
            return context;
        }
        state.current = context.clone();
        state.listeners.values().cloned().collect::<Vec<_>>()
    };

    for listener in listeners {
        listener(context.clone());
    }

    context
}

fn publish_if_changed(next: RegionalContext) -> RegionalContext {
    let listeners = {
        let mut state = runtime()
            .state
            .lock()
            .expect("waterui-locale regional runtime mutex poisoned");

        if state.current == next {
            return next;
        }

        state.current = next.clone();
        state.listeners.values().cloned().collect::<Vec<_>>()
    };

    for listener in listeners {
        listener(next.clone());
    }

    next
}

fn detect_system_context() -> RegionalContext {
    let mut preferred_languages: Vec<String> = sys_locale::get_locales()
        .map(|locale| normalize_locale_tag(&locale))
        .filter(|locale| !locale.is_empty())
        .collect();

    if preferred_languages.is_empty()
        && let Some(locale) = sys_locale::get_locale()
    {
        let locale = normalize_locale_tag(&locale);
        if !locale.is_empty() {
            preferred_languages.push(locale);
        }
    }

    if preferred_languages.is_empty() {
        preferred_languages.push("en-US".to_string());
    }

    let locale_tag = preferred_languages[0].clone();
    // ESP-IDF has no OS timezone database; the asymmetry is explicit.
    #[cfg(target_os = "espidf")]
    let timezone = "UTC".to_string();
    #[cfg(not(target_os = "espidf"))]
    let timezone = iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".to_string());

    RegionalContext::new(locale_tag, preferred_languages, timezone)
}

fn normalize_locale_tag(raw: &str) -> String {
    let cleaned = raw.trim().replace('_', "-");
    if cleaned.is_empty() {
        return "en-US".to_string();
    }

    let parts: Vec<&str> = cleaned.split('-').filter(|part| !part.is_empty()).collect();
    if parts.is_empty() {
        return "en-US".to_string();
    }

    let mut normalized = Vec::with_capacity(parts.len());

    for (index, part) in parts.into_iter().enumerate() {
        let canonical = if index == 0 {
            part.to_ascii_lowercase()
        } else if part.len() == 4 && part.chars().all(|c| c.is_ascii_alphabetic()) {
            let mut chars = part.chars();
            let first = chars
                .next()
                .map(|c| c.to_ascii_uppercase())
                .unwrap_or_default();
            let rest = chars.as_str().to_ascii_lowercase();
            format!("{first}{rest}")
        } else if (part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()))
            || (part.len() == 3 && part.chars().all(|c| c.is_ascii_digit()))
        {
            part.to_ascii_uppercase()
        } else {
            part.to_string()
        };

        normalized.push(canonical);
    }

    normalized.join("-")
}

fn extract_region(locale_tag: &str) -> Option<String> {
    for subtag in locale_tag.split('-').skip(1) {
        let is_region_alpha = subtag.len() == 2 && subtag.chars().all(|c| c.is_ascii_alphabetic());
        let is_region_numeric = subtag.len() == 3 && subtag.chars().all(|c| c.is_ascii_digit());

        if is_region_alpha || is_region_numeric {
            return Some(subtag.to_ascii_uppercase());
        }

        if subtag.len() == 1 {
            break;
        }
    }

    None
}

impl Extractor for RegionalContext {
    fn extract(env: &Environment) -> Result<Self, anyhow::Error> {
        if let Some(locale) = env.get::<Binding<Locale>>() {
            return Ok(current_settings().with_locale(&locale.get()));
        }

        if let Some(context) = env.get::<Self>().cloned() {
            return Ok(context);
        }

        if let Some(locale) = env.get::<Locale>().cloned() {
            return Ok(current_settings().with_locale(&locale));
        }

        Ok(current_settings())
    }
}

#[cfg(test)]
mod tests {
    use super::{current_settings, extract_region, normalize_locale_tag, register_listener};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    #[test]
    fn normalize_locale_tag_handles_separator_and_casing() {
        assert_eq!(normalize_locale_tag("en_us"), "en-US");
        assert_eq!(normalize_locale_tag("ZH-hant-hk"), "zh-Hant-HK");
        assert_eq!(normalize_locale_tag("  fr-FR  "), "fr-FR");
    }

    #[test]
    fn extract_region_reads_alpha_and_numeric_regions() {
        assert_eq!(extract_region("en-US"), Some("US".to_string()));
        assert_eq!(extract_region("es-419"), Some("419".to_string()));
        assert_eq!(extract_region("zh-Hant"), None);
        assert_eq!(extract_region("en-US-u-hc-h23"), Some("US".to_string()));
    }

    #[test]
    fn unregister_drops_listener_after_runtime_mutex_is_released() {
        struct DropReadsRuntime(Arc<AtomicBool>);

        impl Drop for DropReadsRuntime {
            fn drop(&mut self) {
                let _ = current_settings();
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let dropped = Arc::new(AtomicBool::new(false));
        let probe = DropReadsRuntime(Arc::clone(&dropped));
        let handle = register_listener(move |_| {
            let _ = &probe;
        });

        handle.unregister();

        assert!(dropped.load(Ordering::SeqCst));
    }
}
