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
/// The media query the page answers for reduced motion.
const REDUCED_MOTION_QUERY: &str = "(prefers-reduced-motion: reduce)";
/// The event a media query list raises when its result moves.
const CHANGE_EVENT: &str = "change";

/// The page's preferred colour scheme and reduced-motion preference as signals,
/// kept current for as long as this value lives.
pub struct SystemAppearance {
    scheme: Binding<ColorScheme>,
    reduce_motion: Binding<bool>,
    /// Live media queries and their change listeners; missing when the page
    /// offers no media query.
    observations: Vec<Observation>,
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
            .field("reduce_motion", &self.reduce_motion.snapshot())
            .finish_non_exhaustive()
    }
}

impl SystemAppearance {
    /// Reads the page's preferred colour scheme and reduced-motion setting and
    /// observes their changes.
    ///
    /// A page with no `window`, no `matchMedia`, or one that rejects a query
    /// exposes no preference: the failure is logged and the setting keeps its
    /// default (`Light` / `false`).
    pub fn observe() -> Self {
        let mut observations = Vec::with_capacity(2);
        let scheme = observe_media_binding(
            DARK_SCHEME_QUERY,
            ColorScheme::Light,
            color_scheme_of,
            &mut observations,
        );
        let reduce_motion = observe_media_binding(
            REDUCED_MOTION_QUERY,
            false,
            MediaQueryList::matches,
            &mut observations,
        );
        Self {
            scheme,
            reduce_motion,
            observations,
        }
    }

    /// The colour-scheme signal following the page's preferred scheme.
    pub fn scheme(&self) -> Computed<ColorScheme> {
        self.scheme.computed()
    }

    /// The Reduce Motion signal following the page's `prefers-reduced-motion`.
    pub fn reduce_motion(&self) -> Computed<bool> {
        self.reduce_motion.computed()
    }
}

impl Drop for SystemAppearance {
    fn drop(&mut self) {
        for observation in &self.observations {
            if let Err(error) = observation.media_query.remove_event_listener_with_callback(
                CHANGE_EVENT,
                observation.listener.as_ref().unchecked_ref(),
            ) {
                tracing::error!(
                    ?error,
                    "hydrolysis web appearance: failed to remove the media query listener"
                );
            }
        }
    }
}

fn observe_media_binding<T: Clone + PartialEq + 'static>(
    query: &str,
    default: T,
    map: impl Fn(&MediaQueryList) -> T + 'static,
    observations: &mut Vec<Observation>,
) -> Binding<T> {
    let Some(media_query) = media_query(query) else {
        return nami::binding(default);
    };
    let binding = nami::binding(map(&media_query));
    // Re-read the list's `matches` on change and skip redundant writes.
    let listener = super::web_impl::add_event_listener(media_query.as_ref(), CHANGE_EVENT, {
        let media_query = media_query.clone();
        let binding = binding.clone();
        move |_event| {
            let current = map(&media_query);
            if binding.snapshot() != current {
                binding.set(current);
            }
        }
    });
    observations.push(Observation {
        media_query,
        listener,
    });
    binding
}

/// Queries `window.matchMedia(query)`; `None`, logged, when unavailable or rejected.
fn media_query(query: &str) -> Option<MediaQueryList> {
    let Some(window) = web_sys::window() else {
        tracing::error!("hydrolysis web appearance: browser window unavailable");
        return None;
    };
    match window.match_media(query) {
        Ok(Some(query)) => Some(query),
        Ok(None) => {
            tracing::error!("hydrolysis web appearance: the page exposes no matchMedia");
            None
        }
        Err(error) => {
            tracing::error!(
                ?error,
                query,
                "hydrolysis web appearance: matchMedia rejected the query"
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
