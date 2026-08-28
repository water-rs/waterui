//! The `WebView` leaf, as this backend bridges it.
//!
//! Hydrolysis bridges exactly one web engine: the platform's own. On macOS that
//! is `WKWebView`, composed into the winit window as a native subview by the
//! `webview-system` feature. Every other engine — CEF, WPE `WebKit` — is a
//! crate the *application* links and installs, and it reaches the screen as an
//! ordinary `GpuSurface` this renderer knows nothing browser-specific about.
//!
//! A build with neither still renders the component: it occupies its layout slot
//! and publishes its accessibility node, which is the configuration the testing
//! harness runs in.

#[cfg(all(feature = "webview-system", not(hydrolysis_macos_system_webview)))]
compile_error!(
    "the `webview-system` feature selects the macOS WKWebView bridge, which needs \
     `target_os = \"macos\"` and the `winit` feature (WKWebView is composed into the \
     winit window's AppKit view). Enable `winit`, build for macOS, or link a browser \
     engine crate (`waterui-browser-cef`, `waterui-browser-wpe`) in the application \
     instead."
);

use std::cell::RefCell;
use std::rc::Rc;

use waterui_core::Environment;
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_webview::WebView;

#[cfg(hydrolysis_macos_system_webview)]
mod macos;

#[cfg(hydrolysis_macos_system_webview)]
pub use macos::MacSystemWebViewController;

use crate::renderer::{HydroNativeView, HydroState, HydrolysisRenderer, WidgetRenderContext};

/// Retains the semantic WebView and its selected native engine for the node lifetime.
pub(crate) struct WebViewRenderState {
    _source: WebView,
    #[cfg(hydrolysis_macos_system_webview)]
    native: macos::MacSystemWebViewHandle,
    /// Where `WaterUI` content covers the native view, republished every frame
    /// and read by the AppKit view host when it hit-tests. Owned here so it
    /// lives exactly as long as the node the native view belongs to.
    #[cfg(hydrolysis_macos_system_webview)]
    occlusion: Rc<RefCell<Vec<vello::kurbo::Rect>>>,
}

impl WebViewRenderState {
    #[cfg(hydrolysis_macos_system_webview)]
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

    /// Built when this backend bridges no platform web engine.
    ///
    /// The component still occupies its layout slot and still reports itself to
    /// the accessibility tree; only the web content is absent. A build without a
    /// bridge is a missing feature, not a crash, and it is the configuration the
    /// testing harness runs in.
    #[cfg(not(hydrolysis_macos_system_webview))]
    pub(crate) fn from_view(view: WebView, env: &Environment) -> Self {
        let _ = env;
        Self { _source: view }
    }

    pub(crate) fn prebuild(&mut self, renderer: &mut HydrolysisRenderer, env: &Environment) {
        let _ = (renderer, env);
    }
}

impl HydroNativeView for WebView {
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        let _ = (state, view, env);
        LayoutSize::zero()
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        let _ = (state, view, env);
        ViewDimensions::new(LayoutSize::new(
            proposal.width.unwrap_or(0.0),
            proposal.height.unwrap_or(0.0),
        ))
    }
}

/// Measures a retained webview leaf from its [`WebViewRenderState`]: the webview
/// sizes itself to its composed content, mirroring the dispatch path's `dimensions`.
pub(crate) fn measure_webview_node(
    state: &WebViewRenderState,
    proposal: ProposalSize,
    hydro: &mut HydroState,
    _env: &Environment,
) -> ViewDimensions {
    let _ = (state, hydro);
    ViewDimensions::new(LayoutSize::new(
        proposal.width.unwrap_or(0.0),
        proposal.height.unwrap_or(0.0),
    ))
}

/// Renders a retained webview leaf every flush: flushes the composed content
/// sub-view at the node's bounds. The content's own dispatch drives accessibility
/// (webview a11y is render-driven) and its inner reactive `Text` nodes stay live.
pub(crate) fn render_webview_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<WebViewRenderState>>,
    env: &Environment,
) {
    // Every engine path publishes the same semantic node, exactly once. Page
    // content is opaque to the host accessibility tree, so this is what a screen
    // reader has to work with: the web view's own role, label and bounds.
    super::register_web_surface_accessibility(ctx, env);

    #[cfg(hydrolysis_macos_system_webview)]
    {
        use crate::renderer::transformed_rect;

        let _ = env;
        let bounds = ctx.bounds;
        let transform = ctx.render_context().transform;
        let hit_transform = ctx.hit_transform;
        let (native, occlusion) = {
            let state = state.borrow();
            (state.native.native_view(), Rc::clone(&state.occlusion))
        };
        let renderer = ctx.renderer_mut();
        // WebKit hit-tests the `WKWebView` itself, so Hydrolysis has to tell the
        // view host where its own content sits on top; without this a snackbar
        // or dialog over the page was visible and inert.
        renderer.register_native_view_occlusion(
            transformed_rect(hit_transform, bounds),
            Rc::clone(&occlusion),
        );
        renderer.record_native_view_layer(native, transform, bounds, occlusion);
    }
    // No bridge: the semantic node registered above is the whole rendering, and
    // it has already happened. Registering it a second time here published two
    // nodes for one web view, which a screen reader reads as two.
    #[cfg(not(hydrolysis_macos_system_webview))]
    let _ = state;
}

#[cfg(hydrolysis_macos_system_webview)]
pub(crate) fn install_controller(env: &mut Environment) {
    macos::install(env);
}

// No `install_controller` without the macOS bridge. A build that bridges no
// platform engine installs no controller, and a `WebView` created in it fails
// where it is created unless the application linked a browser engine crate,
// whose own `install` supplies the controller. An application that wants the
// contentless web view on purpose asks for it by name, with
// `WebViewController::without_engine()`; the rendering path above then publishes
// its accessibility node and nothing else.
