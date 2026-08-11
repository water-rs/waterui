#[cfg(hydrolysis_linux_wpe_webview)]
use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;
#[cfg(hydrolysis_linux_wpe_webview)]
use std::time::Instant;

use waterui_core::Environment;
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
#[cfg(hydrolysis_linux_wpe_webview)]
use waterui_graphics::gpu_surface::GpuSurface;
use waterui_webview::WebView;

#[cfg(hydrolysis_macos_system_webview)]
mod macos;

#[cfg(hydrolysis_linux_wpe_webview)]
use crate::platform::{KeyCode, Modifiers, NativeKey, PointerButton};
#[cfg(hydrolysis_linux_wpe_webview)]
use crate::renderer::BrowserInputHandler;
#[cfg(hydrolysis_linux_wpe_webview)]
use crate::renderer::{EmbeddedGpuSurfaceRuntime, GpuSurfaceSource};
use crate::renderer::{HydroNativeView, HydroState, HydrolysisRenderer, WidgetRenderContext};

#[cfg(hydrolysis_linux_wpe_webview)]
const WPE_MODIFIER_CONTROL: u32 = 1 << 0;
#[cfg(hydrolysis_linux_wpe_webview)]
const WPE_MODIFIER_SHIFT: u32 = 1 << 1;
#[cfg(hydrolysis_linux_wpe_webview)]
const WPE_MODIFIER_ALT: u32 = 1 << 2;
#[cfg(hydrolysis_linux_wpe_webview)]
const WPE_MODIFIER_META: u32 = 1 << 3;
#[cfg(hydrolysis_linux_wpe_webview)]
const WPE_BUTTON_PRIMARY: u32 = 1 << 8;
#[cfg(hydrolysis_linux_wpe_webview)]
const WPE_BUTTON_MIDDLE: u32 = 1 << 9;
#[cfg(hydrolysis_linux_wpe_webview)]
const WPE_BUTTON_SECONDARY: u32 = 1 << 10;
#[cfg(hydrolysis_linux_wpe_webview)]
const WPE_BUTTON_BACK: u32 = 1 << 11;
#[cfg(hydrolysis_linux_wpe_webview)]
const WPE_BUTTON_FORWARD: u32 = 1 << 12;

#[cfg(hydrolysis_linux_wpe_webview)]
struct WpeInputHandler {
    page: waterui_browser_wpe::WpePage,
    modifiers: Cell<u32>,
    buttons: Cell<u32>,
    last_pointer: Cell<Option<vello::kurbo::Point>>,
    started_at: Instant,
}

#[cfg(hydrolysis_linux_wpe_webview)]
impl WpeInputHandler {
    fn new(page: waterui_browser_wpe::WpePage) -> Self {
        Self {
            page,
            modifiers: Cell::new(0),
            buttons: Cell::new(0),
            last_pointer: Cell::new(None),
            started_at: Instant::now(),
        }
    }

    fn time_ms(&self) -> u32 {
        let range = u128::from(u32::MAX) + 1;
        u32::try_from(self.started_at.elapsed().as_millis() % range)
            .expect("WPE timestamp modulo u32 must fit")
    }

    fn event_modifiers(&self) -> u32 {
        self.modifiers.get() | self.buttons.get()
    }

    fn button(button: PointerButton) -> (waterui_browser_wpe::PointerButton, u32) {
        match button {
            PointerButton::Primary => (
                waterui_browser_wpe::PointerButton::Primary,
                WPE_BUTTON_PRIMARY,
            ),
            PointerButton::Middle => (
                waterui_browser_wpe::PointerButton::Middle,
                WPE_BUTTON_MIDDLE,
            ),
            PointerButton::Secondary => (
                waterui_browser_wpe::PointerButton::Secondary,
                WPE_BUTTON_SECONDARY,
            ),
            PointerButton::Back => (waterui_browser_wpe::PointerButton::Back, WPE_BUTTON_BACK),
            PointerButton::Forward => (
                waterui_browser_wpe::PointerButton::Forward,
                WPE_BUTTON_FORWARD,
            ),
            PointerButton::Other(value) => {
                panic!("WPE does not support pointer button {value}")
            }
        }
    }
}

#[cfg(hydrolysis_linux_wpe_webview)]
impl BrowserInputHandler for WpeInputHandler {
    fn set_focus(&self, focused: bool) {
        self.page.set_focus(focused);
    }

    fn set_modifiers(&self, modifiers: Modifiers) {
        let mut value = 0;
        if modifiers.control {
            value |= WPE_MODIFIER_CONTROL;
        }
        if modifiers.shift {
            value |= WPE_MODIFIER_SHIFT;
        }
        if modifiers.alt {
            value |= WPE_MODIFIER_ALT;
        }
        if modifiers.super_key {
            value |= WPE_MODIFIER_META;
        }
        self.modifiers.set(value);
    }

    fn pointer_move(&self, position: vello::kurbo::Point) {
        let previous = self.last_pointer.replace(Some(position));
        let delta = previous.map_or((0.0, 0.0), |previous| {
            (position.x - previous.x, position.y - previous.y)
        });
        self.page.pointer_move(
            position.x,
            position.y,
            delta.0,
            delta.1,
            self.event_modifiers(),
            self.time_ms(),
        );
    }

    fn pointer_button(&self, pressed: bool, button: PointerButton, position: vello::kurbo::Point) {
        let (button, mask) = Self::button(button);
        let buttons = if pressed {
            self.buttons.get() | mask
        } else {
            self.buttons.get() & !mask
        };
        self.buttons.set(buttons);
        self.page.pointer_button(
            pressed,
            button,
            position.x,
            position.y,
            self.event_modifiers(),
            self.time_ms(),
        );
    }

    fn scroll(
        &self,
        position: vello::kurbo::Point,
        delta_x: f32,
        delta_y: f32,
        is_line_delta: bool,
    ) {
        self.page.scroll(
            position.x,
            position.y,
            f64::from(delta_x),
            f64::from(delta_y),
            !is_line_delta,
            false,
            self.event_modifiers(),
            self.time_ms(),
        );
    }

    fn key(&self, pressed: bool, _key: &KeyCode, native: Option<NativeKey>) {
        let native = native.expect("WPE keyboard input requires Linux XKB key metadata");
        self.page.key(
            pressed,
            native.keycode,
            native.keyval,
            self.event_modifiers(),
            self.time_ms(),
        );
    }

    fn text_input(&self, _text: &str) {}

    fn commit_text(&self, text: &str) {
        for character in text.chars() {
            let keyval = xkeysym::Keysym::from_char(character).raw();
            self.page
                .key(true, 0, keyval, self.event_modifiers(), self.time_ms());
            self.page
                .key(false, 0, keyval, self.event_modifiers(), self.time_ms());
        }
    }
}

/// Retains the semantic WebView and its selected native engine for the node lifetime.
pub(crate) struct WebViewRenderState {
    _source: WebView,
    #[cfg(hydrolysis_macos_system_webview)]
    native: macos::MacSystemWebViewHandle,
    #[cfg(hydrolysis_linux_wpe_webview)]
    gpu: Rc<RefCell<EmbeddedGpuSurfaceRuntime>>,
    #[cfg(hydrolysis_linux_wpe_webview)]
    viewport: waterui_browser_wpe::WpeViewport,
    #[cfg(hydrolysis_linux_wpe_webview)]
    input: Rc<WpeInputHandler>,
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
        let surface = GpuSurface::new(waterui_browser_wpe::WpeGpuView::with_external_input(
            handle.page().clone(),
            viewport.clone(),
        ));
        let input = Rc::new(WpeInputHandler::new(handle.page().clone()));
        Self {
            _source: view,
            gpu: Rc::new(RefCell::new(EmbeddedGpuSurfaceRuntime::new(surface, env))),
            viewport,
            input,
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

/// Publishes the web view's own accessibility node.
///
/// The engine renders page content into a texture or a native subview, neither of
/// which the host tree can see into, so the component itself must appear. The role
/// defaults to a group and is overridden by `a11y_role`; the label comes from
/// `a11y_label`.
fn register_webview_accessibility(ctx: &mut WidgetRenderContext<'_>, env: &Environment) {
    #[cfg(feature = "accessibility")]
    {
        use accesskit::{Node as AccessibilityNode, Role as AccessibilityNodeRole};

        use crate::renderer::transformed_rect;

        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        let renderer = ctx.renderer_mut();
        let mut node = AccessibilityNode::new(
            renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Group),
        );
        if let Some(label) = renderer.resolve_accessibility_label(env, None) {
            node.set_label(label);
        }
        let _ = renderer.register_accessibility_node(node, bounds, env, None);
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = (ctx, env);
    }
}

/// Renders a retained webview leaf every flush: flushes the composed content
/// sub-view at the node's bounds. The content's own dispatch drives accessibility
/// (webview a11y is render-driven) and its inner reactive `Text` nodes stay live.
pub(crate) fn render_webview_node(
    ctx: &mut WidgetRenderContext<'_>,
    state: &Rc<RefCell<WebViewRenderState>>,
    env: &Environment,
) {
    // Every engine path publishes the same semantic node. Page content is opaque to
    // the host accessibility tree, so this is what a screen reader has to work
    // with: the web view's own role, label and bounds.
    register_webview_accessibility(ctx, env);

    #[cfg(hydrolysis_macos_system_webview)]
    {
        let _ = env;
        let bounds = ctx.bounds;
        let transform = ctx.render_context().transform;
        let native = state.borrow().native.native_view();
        ctx.renderer_mut()
            .record_native_view_layer(native, transform, bounds);
    }
    #[cfg(hydrolysis_linux_wpe_webview)]
    {
        let _ = env;
        let bounds = ctx.bounds;
        let transform = ctx.render_context().transform;
        let hit_transform = ctx.hit_transform;
        let state = state.borrow();
        state
            .viewport
            .set_scale(transform.determinant().abs().sqrt());
        let gpu = Rc::clone(&state.gpu);
        let input: Rc<dyn BrowserInputHandler> = state.input.clone();
        drop(state);
        let renderer = ctx.renderer_mut();
        renderer.register_browser_input_target(bounds, hit_transform, input);
        renderer.push_gpu_surface_layer(GpuSurfaceSource::Owned(gpu), transform, bounds);
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
    #[cfg(not(any(
        hydrolysis_macos_system_webview,
        hydrolysis_linux_wpe_webview,
        hydrolysis_cef_webview
    )))]
    {
        let _ = state;
        register_webview_accessibility(ctx, env);
    }
}

#[cfg(hydrolysis_macos_system_webview)]
pub(crate) fn install_controller(env: &mut Environment) {
    macos::install(env);
}

#[cfg(hydrolysis_linux_wpe_webview)]
pub(crate) fn install_controller(env: &mut Environment) {
    use waterui_webview::WebViewController;

    let controller = waterui_browser_wpe::WpeController::packaged();
    env.insert(WebViewController::new(controller));
}

// No `install_controller` without an engine: the caller is gated on
// `hydrolysis_webview`, so a build with no engine never asks for a controller and
// a `WebView` in the tree renders as its accessible, contentless placeholder.
