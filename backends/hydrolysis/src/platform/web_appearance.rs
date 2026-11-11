//! The application's light or dark appearance on the web, as the
//! colour-scheme signal the page supplies to the theme tokens.
//!
//! The source of truth is `window.matchMedia("(prefers-color-scheme:
//! dark)")`: its `matches` seeds the scheme, so the first frame already
//! draws in the page's scheme, and the media query's `change` event writes
//! every later one into the same [`Binding`], so the framework default
//! tokens — maps over the scheme — re-resolve per slot and the views
//! reading them repaint, with no tree rebuild.
//!
//! The observation mirrors `macos_appearance` for the winit host.

use nami::{Binding, Computed, Signal, SignalExt as _};
use wasm_bindgen::{JsCast, closure::Closure};
use waterui::theme::ColorScheme;
use web_sys::{Event, MediaQueryList};

/// The media query the page answers for the dark scheme.
const DARK_SCHEME_QUERY: &str = "(prefers-color-scheme: dark)";
/// The event a media query list raises when its result moves.
const CHANGE_EVENT: &str = "change";

/// The page's preferred colour scheme as a signal, kept current for as
/// long as this value lives.
pub struct SystemAppearance {
    scheme: Binding<ColorScheme>,
    /// The observed media query and its change listener; `None` when the page
    /// offers no media query, leaving the scheme at its Light seed.
    observation: Option<Observation>,
}

/// A live media query and the listener registered on it.
struct Observation {
    media_query: MediaQueryList,
    listener: Closure<dyn FnMut(Event)>,
}

impl core::fmt::Debug for SystemAppearance {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SystemAppearance")
            .field("scheme", &self.scheme.snapshot())
            .finish_non_exhaustive()
    }
}

impl SystemAppearance {
    /// Reads the page's preferred colour scheme and observes its changes.
    ///
    /// A page with no `window`, no `matchMedia`, or one that rejects the
    /// query exposes no preference: the failure is logged and the scheme
    /// stays Light.
    pub fn observe() -> Self {
        let Some(media_query) = dark_scheme_query() else {
            return Self {
                scheme: nami::binding(ColorScheme::Light),
                observation: None,
            };
        };
        let scheme = nami::binding(color_scheme_of(&media_query));
        // The event's own `matches` is the list's, re-read here; the
        // binding takes it only when it moved, so a redundant delivery
        // notifies nothing.
        let listener = super::web_impl::add_event_listener(media_query.as_ref(), CHANGE_EVENT, {
            let media_query = media_query.clone();
            let scheme = scheme.clone();
            move |_event| {
                let current = color_scheme_of(&media_query);
                if scheme.snapshot() != current {
                    scheme.set(current);
                }
            }
        });
        Self {
            scheme,
            observation: Some(Observation {
                media_query,
                listener,
            }),
        }
    }

    /// The colour-scheme signal following the page's preferred scheme.
    pub fn scheme(&self) -> Computed<ColorScheme> {
        self.scheme.computed()
    }
}

impl Drop for SystemAppearance {
    fn drop(&mut self) {
        let Some(observation) = &self.observation else {
            return;
        };
        if let Err(error) = observation.media_query.remove_event_listener_with_callback(
            CHANGE_EVENT,
            observation.listener.as_ref().unchecked_ref(),
        ) {
            tracing::error!(
                ?error,
                "hydrolysis web appearance: failed to remove the prefers-color-scheme listener"
            );
        }
    }
}

/// The page's dark-scheme media query; `None`, logged, when the page exposes
/// no `window` or `matchMedia` or rejects the query.
fn dark_scheme_query() -> Option<MediaQueryList> {
    let Some(window) = web_sys::window() else {
        tracing::error!("hydrolysis web appearance: browser window unavailable");
        return None;
    };
    match window.match_media(DARK_SCHEME_QUERY) {
        Ok(Some(query)) => Some(query),
        Ok(None) => {
            tracing::error!("hydrolysis web appearance: the page exposes no matchMedia");
            None
        }
        Err(error) => {
            tracing::error!(
                ?error,
                "hydrolysis web appearance: matchMedia rejected the prefers-color-scheme query"
            );
            None
        }
    }
}

fn color_scheme_of(query: &MediaQueryList) -> ColorScheme {
    if query.matches() {
        ColorScheme::Dark
    } else {
        ColorScheme::Light
    }
}
