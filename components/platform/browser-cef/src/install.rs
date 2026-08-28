//! Where an application links CEF into its own environment.
//!
//! Which browser engine draws a `WebView` is the application's choice, not the
//! renderer's: an app that wants Chromium depends on this crate and calls
//! [`install`] from its composition root. The renderer stays engine-agnostic —
//! it draws the [`GpuSurface`] this hook returns exactly like any other — and a
//! build that never asks for CEF links none of it.

use waterui_core::{AnyView, Environment, Metadata, Retain, view::Hook};
use waterui_graphics::gpu_surface::GpuSurface;

use crate::{CefRuntime, CefRuntimeConfiguration};

/// Returns the process-owned CEF runtime, starting one if the environment has
/// none, and makes sure its message pump is running.
fn ensure_runtime(env: &mut Environment) -> CefRuntime {
    let runtime = env.get::<CefRuntime>().cloned().unwrap_or_else(|| {
        let runtime = CefRuntime::initialize(CefRuntimeConfiguration::packaged());
        env.insert(runtime.clone());
        runtime
    });
    runtime.start_message_pump();
    runtime
}

/// Installs the CEF realization of the standard [`WebView`](waterui_webview::WebView).
///
/// Call this from the application's composition root, before the view tree is
/// built:
///
/// ```ignore
/// pub fn app(mut env: Environment) -> App {
///     waterui_browser_cef::install(&mut env);
///     App::new(root, env)
/// }
/// ```
///
/// It starts (or reuses) the packaged CEF runtime, supplies a
/// [`WebViewController`](waterui_webview::WebViewController) unless the
/// application already installed one of its own, and registers the hook that
/// draws every `WebView` into a GPU surface.
///
/// # Panics
///
/// Panics when a `WebView` realization is already installed. Two engines cannot
/// both draw one component, and picking the winner silently is how an
/// application ends up running an engine it did not choose: an explicit install
/// colliding with another explicit install is a configuration error. A backend
/// that bridges a platform web engine is not affected — it takes the component
/// by type before the hook is ever consulted.
#[cfg(feature = "webview")]
pub fn install(env: &mut Environment) {
    use waterui_webview::{WebView, WebViewController, web_surface_semantics};

    use crate::CefWebViewHandle;

    assert!(
        env.get::<Hook<WebView>>().is_none(),
        "a WebView realization is already installed; an application selects exactly one browser engine"
    );
    let runtime = ensure_runtime(env);
    // The controller is a default, not an override: an application or test that
    // opened its pages with a controller of its own keeps it.
    if env.get::<WebViewController>().is_none() {
        env.insert(runtime.webview_controller());
    }
    env.insert_hook::<WebView, AnyView>(|env, webview| {
        let page = webview
            .handle()
            .downcast_ref::<CefWebViewHandle>()
            .expect(
                "the CEF WebView realization was handed a page from another engine; \
                 the WebViewController in this environment is not CEF's",
            )
            .page()
            .clone();
        // The CEF view takes its own input: it reports `wants_input_events`, so
        // a renderer routes what lands on this layer straight into this crate's
        // adapter and owns no Chromium semantics.
        let surface = GpuSurface::new(crate::gpu_view_with_input(page));
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

/// Installs the CEF realization of
/// [`ChromiumView`](waterui_chromium::ChromiumView).
///
/// The full Chromium component is separate from the standard `WebView` on
/// purpose, and so is its install: an application that wants both calls both,
/// and they share one runtime and one message pump.
///
/// # Panics
///
/// Panics when a `ChromiumView` realization is already installed, for the same
/// reason [`install`] does.
#[cfg(feature = "chromium")]
pub fn install_chromium(env: &mut Environment) {
    use waterui_chromium::{ChromiumController, ChromiumView, PageMode};
    use waterui_webview::web_surface_semantics;

    use crate::CefPageHandle;

    assert!(
        env.get::<Hook<ChromiumView>>().is_none(),
        "a ChromiumView realization is already installed; an application selects exactly one Chromium runtime"
    );
    let runtime = ensure_runtime(env);
    if env.get::<ChromiumController>().is_none() {
        env.insert(runtime.chromium_controller());
    }
    env.insert_hook::<ChromiumView, AnyView>(|env, chromium| {
        assert_eq!(
            chromium.page().mode(),
            PageMode::Visible,
            "a headless Chromium page cannot be rendered as a ChromiumView"
        );
        let page = chromium
            .page()
            .handle()
            .downcast_ref::<CefPageHandle>()
            .expect(
                "the Chromium realization was handed a page from another runtime; \
                 the ChromiumController in this environment is not CEF's",
            )
            .clone();
        let surface = GpuSurface::new(crate::gpu_view_with_input(page));
        AnyView::new(Metadata::new(
            web_surface_semantics(env, surface),
            Retain::new(chromium),
        ))
    });
}
