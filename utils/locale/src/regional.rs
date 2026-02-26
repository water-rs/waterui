//! Shared locale runtime for WaterUI.

use core::str::FromStr;
use std::collections::BTreeMap;
use std::env;
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};

use crate::locale::{Locale, locales};

type Listener = Arc<dyn Fn(&RegionalContext) + Send + Sync + 'static>;

/// Runtime locale context shared by reactive localized text rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionalContext {
    locale: Locale,
    locale_tag: String,
}

impl RegionalContext {
    fn from_locale(locale: Locale) -> Self {
        let locale_tag = locale.canonical_tag();
        Self { locale, locale_tag }
    }

    /// Returns the canonical BCP 47 locale tag.
    #[must_use]
    pub fn locale_tag(&self) -> &str {
        &self.locale_tag
    }

    /// Returns the parsed locale value.
    #[must_use]
    pub fn locale(&self) -> &Locale {
        &self.locale
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

impl std::error::Error for InvalidLocaleTag {}

/// Handle that keeps a listener subscribed to locale changes.
///
/// Dropping this value automatically unregisters the listener.
pub struct ListenerHandle {
    id: usize,
    registry: &'static Registry,
}

impl fmt::Debug for ListenerHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ListenerHandle")
            .field("id", &self.id)
            .finish()
    }
}

impl Drop for ListenerHandle {
    fn drop(&mut self) {
        self.registry.remove_listener(self.id);
    }
}

struct Registry {
    state: Mutex<State>,
}

struct State {
    next_listener_id: usize,
    context: RegionalContext,
    listeners: BTreeMap<usize, Listener>,
    explicit_locale_set: bool,
}

impl Registry {
    fn new() -> Self {
        let initial = detect_system_locale().unwrap_or(locales::EN_US);
        Self {
            state: Mutex::new(State {
                next_listener_id: 0,
                context: RegionalContext::from_locale(initial),
                listeners: BTreeMap::new(),
                explicit_locale_set: false,
            }),
        }
    }

    fn current_settings(&self) -> RegionalContext {
        self.state
            .lock()
            .expect("regional runtime mutex poisoned")
            .context
            .clone()
    }

    fn add_listener(&'static self, listener: Listener) -> ListenerHandle {
        let mut state = self.state.lock().expect("regional runtime mutex poisoned");
        let id = state.next_listener_id;
        state.next_listener_id = state
            .next_listener_id
            .checked_add(1)
            .expect("regional listener id overflow");
        state.listeners.insert(id, listener);
        ListenerHandle { id, registry: self }
    }

    fn remove_listener(&self, id: usize) {
        let mut state = self.state.lock().expect("regional runtime mutex poisoned");
        state.listeners.remove(&id);
    }

    fn set_locale(&self, locale: Locale, explicit: bool) {
        let context = RegionalContext::from_locale(locale);
        let listeners = {
            let mut state = self.state.lock().expect("regional runtime mutex poisoned");
            if state.context == context {
                if explicit {
                    state.explicit_locale_set = true;
                }
                return;
            }
            if explicit {
                state.explicit_locale_set = true;
            }
            state.context = context.clone();
            state.listeners.values().cloned().collect::<Vec<_>>()
        };

        for listener in listeners {
            listener(&context);
        }
    }

    fn refresh_default(&self) {
        let detected = {
            let state = self.state.lock().expect("regional runtime mutex poisoned");
            if state.explicit_locale_set {
                return;
            }
            detect_system_locale()
        };

        if let Some(locale) = detected {
            self.set_locale(locale, false);
        }
    }

    #[cfg(test)]
    fn reset_for_tests(&self) {
        let initial = detect_system_locale().unwrap_or(locales::EN_US);
        let mut state = self.state.lock().expect("regional runtime mutex poisoned");
        state.next_listener_id = 0;
        state.context = RegionalContext::from_locale(initial);
        state.listeners.clear();
        state.explicit_locale_set = false;
    }
}

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(Registry::new)
}

fn detect_system_locale() -> Option<Locale> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(|key| env::var(key).ok())
        .filter_map(|value| normalize_locale_env_value(&value))
        .find_map(|value| Locale::from_str(&value).ok())
}

fn normalize_locale_env_value(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "C" || trimmed == "POSIX" {
        return None;
    }
    let without_encoding = trimmed.split('.').next().unwrap_or(trimmed);
    let without_modifier = without_encoding.split('@').next().unwrap_or(without_encoding);
    if without_modifier.is_empty() {
        return None;
    }
    Some(without_modifier.replace('_', "-"))
}

/// Returns the current shared runtime locale settings.
#[must_use]
pub fn current_settings() -> RegionalContext {
    registry().current_settings()
}

/// Sets the shared runtime locale using a BCP 47 tag.
///
/// Returns an error when `tag` is not a valid locale identifier.
pub fn set_locale_tag(tag: impl AsRef<str>) -> Result<(), InvalidLocaleTag> {
    let locale_tag = tag.as_ref();
    let locale = Locale::from_str(locale_tag).map_err(|_| InvalidLocaleTag {
        tag: locale_tag.to_string(),
    })?;
    registry().set_locale(locale, true);
    Ok(())
}

/// Registers a listener that will be invoked whenever the runtime locale changes.
///
/// The returned handle must be kept alive; dropping it unsubscribes the listener.
#[must_use]
pub fn register_listener(
    listener: impl Fn(&RegionalContext) + Send + Sync + 'static,
) -> ListenerHandle {
    registry().add_listener(Arc::new(listener))
}

/// Initializes automatic locale refresh from host defaults.
///
/// This performs a non-overriding refresh from host locale environment when no
/// explicit locale has been set.
pub fn start_auto_refresh_default() {
    registry().refresh_default();
}

#[cfg(test)]
pub(crate) fn reset_runtime_for_tests() {
    registry().reset_for_tests();
}
