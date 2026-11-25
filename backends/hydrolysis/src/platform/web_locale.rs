//! System locale observation on the web via `window`'s `languagechange` event.
//!
//! Replaces the legacy 2-second polling interval with the browser's real
//! `languagechange` event, removing the listener cleanly on [`Drop`].

use core::fmt;

use wasm_bindgen::{JsCast, closure::Closure};
use waterui_locale::regional::refresh_system_locale;
use web_sys::{Event, Window};

use super::web_impl::add_event_listener;

const LANGUAGECHANGE_EVENT: &str = "languagechange";

/// Keeps the browser's `languagechange` event listener registered for as long
/// as this value lives.
pub struct SystemLocale {
    observation: Option<Observation>,
}

struct Observation {
    window: Window,
    listener: Closure<dyn FnMut(Event)>,
}

impl fmt::Debug for SystemLocale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SystemLocale").finish_non_exhaustive()
    }
}

impl SystemLocale {
    /// Subscribes to `window`'s `languagechange` event.
    #[must_use]
    pub fn observe() -> Self {
        let Some(window) = web_sys::window() else {
            tracing::error!(
                "hydrolysis web locale: browser window unavailable; locale changes will not be observed"
            );
            return Self { observation: None };
        };
        let listener = add_event_listener(window.as_ref(), LANGUAGECHANGE_EVENT, |_event| {
            let _ = refresh_system_locale();
        });
        Self {
            observation: Some(Observation { window, listener }),
        }
    }
}

impl Drop for SystemLocale {
    fn drop(&mut self) {
        let Some(observation) = &self.observation else {
            return;
        };
        if let Err(error) = observation.window.remove_event_listener_with_callback(
            LANGUAGECHANGE_EVENT,
            observation.listener.as_ref().unchecked_ref(),
        ) {
            tracing::error!(
                ?error,
                "hydrolysis web locale: failed to remove the languagechange listener"
            );
        }
    }
}
