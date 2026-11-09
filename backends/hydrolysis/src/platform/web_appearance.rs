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
    /// # Panics
    ///
    /// Panics when the page exposes no `window` or no `matchMedia` — a
    /// browser without it cannot host the runner.
    pub fn observe() -> Self {
        let media_query = web_sys::window()
            .expect("hydrolysis web appearance: browser window unavailable")
            .match_media(DARK_SCHEME_QUERY)
            .expect("hydrolysis web appearance: matchMedia rejected the prefers-color-scheme query")
            .expect("hydrolysis web appearance: the page exposes no matchMedia");
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
            media_query,
            listener,
        }
    }

    /// The colour-scheme signal following the page's preferred scheme.
    pub fn scheme(&self) -> Computed<ColorScheme> {
        self.scheme.computed()
    }
}

impl Drop for SystemAppearance {
    fn drop(&mut self) {
        self.media_query
            .remove_event_listener_with_callback(
                CHANGE_EVENT,
                self.listener.as_ref().unchecked_ref(),
            )
            .expect(
                "hydrolysis web appearance: failed to remove the prefers-color-scheme listener",
            );
    }
}

fn color_scheme_of(query: &MediaQueryList) -> ColorScheme {
    if query.matches() {
        ColorScheme::Dark
    } else {
        ColorScheme::Light
    }
}
