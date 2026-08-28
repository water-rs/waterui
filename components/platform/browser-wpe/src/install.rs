//! Where an application links WPE `WebKit` into its own environment.
//!
//! Which browser engine draws a `WebView` is the application's choice, not the
//! renderer's: an app that wants WPE depends on this crate and calls
//! [`install`] from its composition root. The renderer stays engine-agnostic —
//! it draws the [`GpuSurface`] this hook returns exactly like any other — and a
//! build that never asks for WPE links none of it.

use waterui_core::{AnyView, Environment, Metadata, Retain, view::Hook};
use waterui_graphics::gpu_surface::GpuSurface;
use waterui_webview::{WebView, WebViewController, web_surface_semantics};

use crate::{WpeController, WpeWebViewHandle, gpu_view_with_input};

/// Installs the WPE realization of the standard [`WebView`].
///
/// Call this from the application's composition root, before the view tree is
/// built:
///
/// ```ignore
/// pub fn app(mut env: Environment) -> App {
///     waterui_browser_wpe::install(&mut env);
///     App::new(root, env)
/// }
/// ```
///
/// It supplies a [`WebViewController`] backed by the `water`-staged runtime
/// unless the application already installed one of its own — the runtime itself
/// is not loaded until the first page opens — and registers the hook that draws
/// every `WebView` into a GPU surface.
///
/// # Panics
///
/// Panics when a `WebView` realization is already installed. Two engines cannot
/// both draw one component, and picking the winner silently is how an
/// application ends up running an engine it did not choose: an explicit install
/// colliding with another explicit install is a configuration error. A backend
/// that bridges a platform web engine is not affected — it takes the component
/// by type before the hook is ever consulted.
pub fn install(env: &mut Environment) {
    assert!(
        env.get::<Hook<WebView>>().is_none(),
        "a WebView realization is already installed; an application selects exactly one browser engine"
    );
    // The controller is a default, not an override: an application or test that
    // opened its pages with a controller of its own keeps it.
    if env.get::<WebViewController>().is_none() {
        env.insert(WebViewController::new(WpeController::packaged()));
    }
    env.insert_hook::<WebView, AnyView>(|env, webview| {
        let page = webview
            .handle()
            .downcast_ref::<WpeWebViewHandle>()
            .expect(
                "the WPE WebView realization was handed a page from another engine; \
                 the WebViewController in this environment is not WPE's",
            )
            .page()
            .clone();
        // The WPE view takes its own input: it reports `wants_input_events`, so
        // a renderer routes what lands on this layer straight into this crate's
        // adapter and owns no `WPEPlatform` semantics.
        let surface = GpuSurface::new(gpu_view_with_input(page));
        // The semantic `WebView` owns the subscriptions that drive
        // `can_go_back` / `can_go_forward`, the event signal, and the URL and
        // user-agent bindings. Dropping it here would leave a live page whose
        // reactive state had gone dead.
        AnyView::new(Metadata::new(
            web_surface_semantics(env, surface),
            Retain::new(webview),
        ))
    });
}
