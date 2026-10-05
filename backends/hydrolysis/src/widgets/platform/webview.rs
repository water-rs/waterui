//! The `WebView` leaf, as this backend bridges it.
//!
//! Hydrolysis bridges exactly one web engine: the platform's own. On macOS that
//! is `WKWebView`, composed into the winit window as a native subview by the
//! `webview-system` feature. Every other engine — CEF, WPE `WebKit` — is a
//! crate the *application* links and installs as a `Hook<WebView>` realization,
//! which intercepts the component before it reaches this backend.
//!
//! A build with neither draws nothing: a `WebView` that reaches the backend is
//! a missing realization — a programmer error — and panics rather than
//! occupying a layout slot with no page behind it.

// Only macOS can act on this: there the feature names a bridge that exists and
// the diagnostic tells the reader what is missing. Everywhere else the platform
// has no WKWebView to bridge, and the feature selects nothing — which is what
// the module documents above. It has to stay buildable there, because every
// tool that reads a crate whole turns on every feature on Linux: docs.rs,
// `cargo hack`, and the `cargo semver-checks` release-plz runs before it
// publishes. A hard error on those targets makes the crate impossible to
// document and impossible to release, and protects nobody.
#[cfg(all(
    feature = "webview-system",
    target_os = "macos",
    not(hydrolysis_macos_system_webview)
))]
compile_error!(
    "the `webview-system` feature selects the macOS WKWebView bridge, which needs \
     the `winit` feature (WKWebView is composed into the winit window's AppKit \
     view). Enable `winit`, or link a browser engine crate \
     (`waterui-browser-cef`, `waterui-browser-wpe`) in the application instead."
);

#[cfg(hydrolysis_macos_system_webview)]
use std::cell::RefCell;
#[cfg(hydrolysis_macos_system_webview)]
use std::rc::Rc;

use waterui_core::Environment;
use waterui_core::layout::Size as LayoutSize;
#[cfg(hydrolysis_macos_system_webview)]
use waterui_core::layout::{ProposalSize, ViewDimensions};
use waterui_webview::WebView;

#[cfg(hydrolysis_macos_system_webview)]
mod macos;

#[cfg(hydrolysis_macos_system_webview)]
pub use macos::MacSystemWebViewController;

#[cfg(hydrolysis_macos_system_webview)]
use crate::renderer::WidgetRenderContext;
use crate::renderer::{HydroNativeView, HydroState};

/// Retains the semantic WebView and its selected native engine for the node lifetime.
#[cfg(hydrolysis_macos_system_webview)]
pub(crate) struct WebViewRenderState {
    _source: WebView,
    native: macos::MacSystemWebViewHandle,
    /// Where `WaterUI` content covers the native view, republished every frame
    /// and read by the AppKit view host when it hit-tests. Owned here so it
    /// lives exactly as long as the node the native view belongs to.
    occlusion: Rc<RefCell<Vec<kurbo::Rect>>>,
}

#[cfg(hydrolysis_macos_system_webview)]
impl WebViewRenderState {
    pub(crate) fn from_view(view: WebView, env: &Environment) -> Self {
        let _ = env;
        let native = view
            .handle()
            .downcast_ref::<macos::MacSystemWebViewHandle>()
            .unwrap_or_else(|| {
                panic!("Hydrolysis macOS WebView handle does not use the selected system backend")
            })
            .clone();
        Self {
            _source: view,
            native,
            occlusion: Rc::new(RefCell::new(Vec::new())),
        }
    }

    pub(crate) fn prebuild(
        &mut self,
        renderer: &mut crate::renderer::SemanticCore,
        env: &Environment,
    ) {
        let _ = (renderer, env);
    }
}

impl HydroNativeView for WebView {
    #[cfg(hydrolysis_macos_system_webview)]
    fn intrinsic(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        let _ = (state, view, env, theme);
        LayoutSize::zero()
    }

    #[cfg(hydrolysis_macos_system_webview)]
    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        let _ = (state, view, env, theme);
        ViewDimensions::new(LayoutSize::new(
            proposal.width.unwrap_or(0.0),
            proposal.height.unwrap_or(0.0),
        ))
    }

    #[cfg(not(hydrolysis_macos_system_webview))]
    fn intrinsic(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        _theme: &std::rc::Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        crate::renderer::unsupported_webview()
    }
}

/// Measures a retained webview leaf from its [`WebViewRenderState`]: the webview
/// fills its proposal, mirroring the dispatch path's `dimensions`.
#[cfg(hydrolysis_macos_system_webview)]
pub(crate) fn measure_webview_node(
    state: &WebViewRenderState,
    proposal: ProposalSize,
    hydro: &mut HydroState,
    _env: &Environment,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> ViewDimensions {
    let _ = (state, hydro, theme);
    ViewDimensions::new(LayoutSize::new(
        proposal.width.unwrap_or(0.0),
        proposal.height.unwrap_or(0.0),
    ))
}

/// Renders a retained webview leaf every flush: publishes the component's
/// accessibility node and records the native view layer for the AppKit host.
/// Page content is opaque to the host accessibility tree, so the node — the
/// web view's own role, label and bounds — is all a screen reader has.
#[cfg(hydrolysis_macos_system_webview)]
pub(crate) fn render_webview_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<WebViewRenderState>>,
    env: &Environment,
) {
    super::register_web_surface_accessibility(ctx, env);

    let bounds = ctx.bounds;
    let transform = ctx.render_context().transform;

    let (native, occlusion) = {
        let state = state.borrow();
        (state.native.native_view(), Rc::clone(&state.occlusion))
    };
    let renderer = ctx.renderer_mut();
    // WebKit hit-tests the `WKWebView` itself, so Hydrolysis has to tell the
    // view host where its own content sits on top; without this a snackbar
    // or dialog over the page was visible and inert.
    renderer.register_native_view_occlusion(bounds, Rc::clone(&occlusion));
    renderer.record_native_view_layer(native, transform, bounds, occlusion);
}

#[cfg(hydrolysis_macos_system_webview)]
pub(crate) fn install_controller(env: &mut Environment) {
    macos::install(env);
}

// No `install_controller` without the macOS bridge. A build that bridges no
// platform engine installs no controller, so a `WebView` can only be created in
// it under a controller the application installed itself — an engine crate's
// `install`, which also installs the `Hook<WebView>` that intercepts the
// component before this backend sees it. A `WebView` that still gets here has
// nothing to draw it, and panics.
