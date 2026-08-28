//! The `WebView` leaf and the engine that backs it.
//!
//! # Which engine a build gets
//!
//! Selecting a `webview-*` feature does not by itself produce an engine: each
//! one names a bridge that exists on some targets only, and a feature that
//! resolves to nothing used to leave `WebView` silently contentless — or, when
//! `webview-system` was enabled without `winit`, fail to build with a missing
//! `install_controller`. Every declared combination now either resolves to an
//! engine or says here what to enable.

#[cfg(all(feature = "webview-system", not(hydrolysis_macos_system_webview)))]
compile_error!(
    "the `webview-system` feature selects the macOS WKWebView bridge, which needs \
     `target_os = \"macos\"` and the `winit` feature (WKWebView is composed into the \
     winit window's AppKit view). Enable `winit`, build for macOS, or select \
     `webview-cef`/`webview-wpe` instead."
);

#[cfg(all(feature = "webview-wpe", not(hydrolysis_linux_wpe_webview)))]
compile_error!(
    "the `webview-wpe` feature selects the WPE WebKit engine, which exists on Linux \
     only. Build for Linux, or select `webview-cef` (macOS/Linux/Windows) or \
     `webview-system` (macOS)."
);

#[cfg(all(feature = "webview-cef", not(hydrolysis_cef_webview)))]
compile_error!(
    "the `webview-cef` feature selects the CEF runtime, which is supported on macOS, \
     Linux and Windows only."
);

#[cfg(all(
    feature = "webview-default",
    not(any(
        hydrolysis_macos_system_webview,
        hydrolysis_linux_wpe_webview,
        hydrolysis_cef_webview
    ))
))]
compile_error!(
    "the `webview-default` feature resolves to WKWebView on macOS (which also needs \
     the `winit` feature) and to WPE WebKit on Linux; this target has neither. Enable \
     `winit` if you are on macOS, or select `webview-cef`."
);

// Two engines at once is the other way a feature combination goes wrong, and
// `lib.rs` already refuses it; the checks above are only about a request that
// resolves to no engine at all.

use std::cell::RefCell;
use std::rc::Rc;

use waterui_core::Environment;
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
#[cfg(hydrolysis_linux_wpe_webview)]
use waterui_graphics::gpu_surface::GpuSurface;
use waterui_webview::WebView;

#[cfg(hydrolysis_macos_system_webview)]
mod macos;

#[cfg(hydrolysis_macos_system_webview)]
pub use macos::MacSystemWebViewController;

#[cfg(hydrolysis_linux_wpe_webview)]
use crate::renderer::{EmbeddedGpuSurfaceRuntime, GpuSurfaceSource};
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
    #[cfg(hydrolysis_linux_wpe_webview)]
    gpu: Rc<RefCell<EmbeddedGpuSurfaceRuntime>>,
    #[cfg(hydrolysis_linux_wpe_webview)]
    viewport: waterui_browser_wpe::WpeViewport,
    #[cfg(hydrolysis_cef_webview)]
    cef: crate::widgets::platform::browser_cef::CefSurfaceRenderState,
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

    #[cfg(hydrolysis_linux_wpe_webview)]
    pub(crate) fn from_view(view: WebView, env: &Environment) -> Self {
        let handle = view
            .handle()
            .downcast_ref::<waterui_browser_wpe::WpeWebViewHandle>()
            .unwrap_or_else(|| {
                panic!("Hydrolysis Linux WebView handle does not use the selected WPE backend")
            });
        let viewport = waterui_browser_wpe::WpeViewport::new();
        // The WPE view takes its own input: it reports `wants_input_events`, so
        // the renderer routes what lands on this layer straight into the engine
        // crate's adapter and Hydrolysis owns no `WPEPlatform` semantics.
        let surface = GpuSurface::new(waterui_browser_wpe::gpu_view_with_input(
            handle.page().clone(),
            viewport.clone(),
        ));
        Self {
            _source: view,
            gpu: Rc::new(RefCell::new(EmbeddedGpuSurfaceRuntime::new(surface, env))),
            viewport,
        }
    }

    #[cfg(hydrolysis_cef_webview)]
    pub(crate) fn from_view(view: WebView, env: &Environment) -> Self {
        let page = view
            .handle()
            .downcast_ref::<waterui_browser_cef::CefWebViewHandle>()
            .unwrap_or_else(|| {
                panic!("Hydrolysis WebView handle does not use the selected CEF backend")
            })
            .page()
            .clone();
        Self {
            _source: view,
            cef: crate::widgets::platform::browser_cef::CefSurfaceRenderState::new(page, env),
        }
    }

    /// Built when no web engine feature is selected.
    ///
    /// The component still occupies its layout slot and still reports itself to
    /// the accessibility tree; only the web content is absent. A build without an
    /// engine is a missing feature, not a crash, and it is the configuration the
    /// testing harness runs in.
    #[cfg(not(any(
        hydrolysis_macos_system_webview,
        hydrolysis_linux_wpe_webview,
        hydrolysis_cef_webview
    )))]
    pub(crate) fn from_view(view: WebView, env: &Environment) -> Self {
        let _ = env;
        Self { _source: view }
    }

    pub(crate) fn prebuild(&mut self, renderer: &mut HydrolysisRenderer, env: &Environment) {
        #[cfg(hydrolysis_linux_wpe_webview)]
        renderer.register_node_gpu_surface(Rc::clone(&self.gpu));
        #[cfg(hydrolysis_cef_webview)]
        self.cef.prebuild(renderer);
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
    #[cfg(hydrolysis_linux_wpe_webview)]
    {
        let _ = env;
        use crate::renderer::transformed_rect;

        let bounds = ctx.bounds;
        let transform = ctx.render_context().transform;
        let hit_transform = ctx.hit_transform;
        let state = state.borrow();
        state
            .viewport
            .set_scale(transform.determinant().abs().sqrt());
        let gpu = Rc::clone(&state.gpu);
        drop(state);
        let renderer = ctx.renderer_mut();
        renderer.register_gpu_surface_input_target(bounds, hit_transform, Rc::clone(&gpu));
        renderer.push_gpu_surface_layer(
            GpuSurfaceSource::Owned(gpu),
            transform,
            bounds,
            transformed_rect(hit_transform, bounds),
        );
    }
    #[cfg(hydrolysis_cef_webview)]
    {
        let _ = env;
        let bounds = ctx.bounds;
        let transform = ctx.render_context().transform;
        let hit_transform = ctx.hit_transform;
        state
            .borrow()
            .cef
            .render(ctx.renderer_mut(), bounds, transform, hit_transform);
    }
    // No engine: the semantic node registered above is the whole rendering, and
    // it has already happened. Registering it a second time here published two
    // nodes for one web view, which a screen reader reads as two.
    #[cfg(not(any(
        hydrolysis_macos_system_webview,
        hydrolysis_linux_wpe_webview,
        hydrolysis_cef_webview
    )))]
    let _ = state;
}

#[cfg(hydrolysis_macos_system_webview)]
pub(crate) fn install_controller(env: &mut Environment) {
    macos::install(env);
}

#[cfg(hydrolysis_linux_wpe_webview)]
pub(crate) fn install_controller(env: &mut Environment) {
    use waterui_webview::WebViewController;

    // The backend supplies the *default* controller. An application or test
    // that already installed one of its own keeps it: overwriting here made the
    // engine this build happens to select silently outrank an explicit choice,
    // which is how test doubles ended up shadowed by the packaged WPE runtime.
    if env.get::<WebViewController>().is_some() {
        return;
    }
    let controller = waterui_browser_wpe::WpeController::packaged();
    env.insert(WebViewController::new(controller));
}

// No `install_controller` without an engine. The caller is gated on the two
// engine cfgs above rather than on `hydrolysis_webview`, which is true for any
// webview feature and so covers engine-less combinations too.
//
// A build with no engine therefore installs no controller, and a `WebView`
// created in it fails where it is created — this backend has no page to show and
// says so, rather than quietly rendering an empty rectangle. An application that
// wants the contentless web view on purpose asks for it by name, with
// `WebViewController::without_engine()`; the rendering path above then publishes
// its accessibility node and nothing else.
