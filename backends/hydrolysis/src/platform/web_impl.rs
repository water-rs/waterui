// The whole module compiles only into wasm32 + `web`: every future here
// awaits handles that are JS objects — DOM elements, the Clipboard, wgpu
// WebGPU devices — which are `!Send` by design on the single-threaded
// target, and none of these functions promise `Send` futures.
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use nami::Signal;
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{
    CompositionEvent, Document, Event, EventTarget, HtmlElement, HtmlInputElement, KeyboardEvent,
    PointerEvent, WheelEvent, Window as BrowserHostWindow,
};

use super::{
    CursorStyle, GpuSurfaceWindow, InputEvent, KeyCode, KeyState, Modifiers, PlatformWindow,
    PointerButton, PointerKind, PresentationSurface as _, ScrollPlatform, TextInputPurpose,
    TextInputState, TouchScrollConfig, WindowState, WuiWindow,
};

/// The touch-scroll physics of the system `window`'s browser runs on, as
/// its `navigator.userAgent` names it.
fn touch_scroll_config_for(window: &BrowserHostWindow) -> TouchScrollConfig {
    let user_agent = window
        .navigator()
        .user_agent()
        .expect("hydrolysis web platform: navigator.userAgent unavailable");
    TouchScrollConfig::for_platform(ScrollPlatform::from_user_agent(&user_agent))
}

#[derive(Clone, Copy)]
struct PendingResize {
    width: u32,
    height: u32,
    scale_factor: f64,
}

/// The page's presentation: the WebGPU device chain the engine renders on,
/// and the engine surface that presents its canvases and the page's hosted
/// elements under the root element through `DomTarget`. Nothing here
/// acquires or presents a frame; the engine does both inside its render.
pub struct BrowserSurface {
    /// The live device generation and its loss budget; a lost device is
    /// recreated on the same instance (water-rs/waterui#2742).
    gpu: crate::platform::RecoverableGpu,
    /// The device generation this surface renders on — a clone of the
    /// context's live generation, adopted in `ensure_live`.
    handles: crate::platform::DeviceLoss,
    /// The element the engine's stacking root is appended to.
    root: HtmlElement,
    /// The drawable size in device pixels.
    size: (u32, u32),
    /// The engine surface and mount, created on the first frame and
    /// replaced when the device chain no longer matches.
    engine_window: Option<
        crate::renderer::CherenkovWindow<
            crate::engine::DomCherenkovSurface,
            crate::text::ParleyEngine,
        >,
    >,
}

impl core::fmt::Debug for BrowserSurface {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BrowserSurface")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl BrowserSurface {
    #[allow(
        clippy::future_not_send,
        reason = "the future runs on the browser main thread via spawn_local; wgpu handles' Send bound varies with Cargo feature unification"
    )]
    pub async fn new(
        root: HtmlElement,
        width: u32,
        height: u32,
    ) -> Result<Self, BrowserGpuUnavailable> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let generation = Self::open_generation(&instance).await?;
        Ok(Self {
            gpu: crate::platform::RecoverableGpu::new(generation.clone()),
            handles: generation,
            root,
            size: (width.max(1), height.max(1)),
            engine_window: None,
        })
    }

    /// Opens a device generation on `instance` — adapter pick, device
    /// request and loss observation. A lost device's recreation re-runs
    /// it on the lost generation's own instance (water-rs/waterui#2742).
    async fn open_generation(
        instance: &wgpu::Instance,
    ) -> Result<crate::platform::DeviceLoss, BrowserGpuUnavailable> {
        // Every adapter a browser hands out presents to a canvas, so the
        // request names no surface: the engine creates its canvases on
        // this device itself.
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .map_err(BrowserGpuUnavailable::NoAdapter)?;

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("hydrolysis-web-device"),
                required_features: super::required_media_features(adapter.features()),
                ..Default::default()
            })
            .await
            .map_err(BrowserGpuUnavailable::NoDevice)?;
        let context_id = super::next_gpu_context_id();
        Ok(crate::platform::DeviceLoss::observe(
            cherenkov_gpu::interop::SharedDevice {
                instance: instance.clone(),
                adapter,
                device,
                queue,
            },
            context_id,
        ))
    }

    /// Adopts the context's live device generation — rebuilding it when
    /// the adopted one was lost — and reports whether a live generation
    /// came out of it (water-rs/waterui#2742). `false` means a failed
    /// recreation left the dead generation in place: the caller reports
    /// the surface lost and the next frame retries under the loss budget,
    /// which panics once it runs out. The engine window is left for the
    /// next render's `renders_on` check to replace — the retained mount
    /// must unmount under the chain that created it.
    #[allow(
        clippy::future_not_send,
        reason = "the future runs on the browser main thread via spawn_local; wgpu handles' Send bound varies with Cargo feature unification"
    )]
    pub(crate) async fn ensure_live(&mut self) -> bool {
        let Some(dead) = self.gpu.take_lost() else {
            self.handles = self.gpu.generation();
            return true;
        };
        let rebuilt = Self::open_generation(&dead.gpu_context().shared_device.instance)
            .await
            .map_err(|error| error.to_string());
        let generation = self.gpu.finish_recovery(&dead, rebuilt);
        if generation.gpu_context().context_id != self.handles.gpu_context().context_id {
            self.handles = generation;
        }
        !self.handles.is_lost()
    }
}

/// Why the page cannot host a Hydrolysis window: Hydrolysis renders through
/// WebGPU, and this browser does not provide it.
#[derive(Debug, thiserror::Error)]
pub enum BrowserGpuUnavailable {
    #[error("This browser does not provide WebGPU, which this app needs to render.")]
    NoAdapter(#[source] wgpu::RequestAdapterError),
    #[error("This browser refused to open a WebGPU device, which this app needs to render.")]
    NoDevice(#[source] wgpu::RequestDeviceError),
}

/// The event the page's bootstrap shows as its startup error: the runtime
/// cannot run, and `detail` says why.
const FATAL_ERROR_EVENT: &str = "waterui:fatal-error";

/// Tells the page that the app cannot run, as a bubbling
/// `waterui:fatal-error` event on the root element whose `detail` is
/// `message`; the page replaces itself with that message.
///
/// The panic hook calls this too, so it reports what it can and never
/// panics: a page missing its root or refusing the event leaves only the
/// console report the hook has already made.
pub fn report_fatal_error(message: &str) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let target: EventTarget = document
        .get_element_by_id(ROOT_ID)
        .map_or_else(|| document.into(), Into::into);
    let init = web_sys::CustomEventInit::new();
    init.set_bubbles(true);
    init.set_detail(&wasm_bindgen::JsValue::from_str(message));
    if let Ok(event) = web_sys::CustomEvent::new_with_event_init_dict(FATAL_ERROR_EVENT, &init) {
        let _ = target.dispatch_event(&event);
    }
}

impl crate::platform::PresentationSurface for BrowserSurface {
    fn adapter(&self) -> &wgpu::Adapter {
        &self.handles.gpu_context().shared_device.adapter
    }

    fn device(&self) -> &wgpu::Device {
        &self.handles.gpu_context().shared_device.device
    }

    fn queue(&self) -> &wgpu::Queue {
        &self.handles.gpu_context().shared_device.queue
    }

    fn device_loss(&self) -> &crate::platform::DeviceLoss {
        &self.handles
    }

    fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The engine resizes its canvases at the next render, from the size
    /// the frame's commit hands the surface.
    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
    }
}

impl BrowserSurface {
    /// The frame's render inputs, the root element the engine presents
    /// under, and the engine window slot it presents through.
    pub(crate) fn render_targets(
        &mut self,
        display_scale: f64,
        base_color: waterui_graphics::draw::WorkingColor,
    ) -> (
        crate::renderer::FrameRenderTarget<'_>,
        &HtmlElement,
        &mut Option<
            crate::renderer::CherenkovWindow<
                crate::engine::DomCherenkovSurface,
                crate::text::ParleyEngine,
            >,
        >,
    ) {
        let context = self.handles.gpu_context();
        (
            crate::renderer::FrameRenderTarget {
                adapter: &context.shared_device.adapter,
                device: &context.shared_device.device,
                queue: &context.shared_device.queue,
                device_loss: self.handles.clone(),
                gpu_context_id: context.context_id,
                shared_device: context.shared_device.clone(),
                display_scale,
                width: self.size.0,
                height: self.size.1,
                base_color,
            },
            &self.root,
            &mut self.engine_window,
        )
    }
}

/// The browser window: the page's root element plus the document, input and
/// pointer plumbing attached to it.
pub struct BrowserWindow {
    host_window: BrowserHostWindow,
    document: Document,
    root: HtmlElement,
    ime_input: HtmlInputElement,
    surface: BrowserSurface,
    pending_events: Rc<RefCell<Vec<InputEvent>>>,
    redraw_requested: Rc<Cell<bool>>,
    /// The root's `IntersectionObserver` report: `true` while the
    /// browser counts it off-screen — scrolled out of view, or
    /// `display:none`'d by an app-driven minimized window state.
    offscreen: Rc<Cell<bool>>,
    scale_factor: Rc<Cell<f64>>,
    pending_resize: Rc<Cell<Option<PendingResize>>>,
    /// The page's safe area, which the window's `WindowSafeArea` installs;
    /// re-read from the probe on every resize.
    safe_area: nami::Binding<waterui_layout::padding::EdgeInsets>,
    /// The runner's `requestAnimationFrame` coalescer — handed out as the
    /// frame-signals wake so a request raised outside a frame schedules
    /// the one it needs.
    schedule_frame: Rc<dyn Fn()>,
    current_cursor_style: CursorStyle,
    /// The touch-scroll physics of the system the browser runs on, read
    /// once from `navigator.userAgent`.
    touch_scroll_config: TouchScrollConfig,
    /// The page background last matched to the app's top edge.
    top_edge_color: Option<waterui_graphics::draw::WorkingColor>,
    /// Held for its lifetime: the observer keeps reporting only while
    /// both halves are alive.
    _intersection_observer: (
        web_sys::IntersectionObserver,
        Closure<dyn FnMut(js_sys::Array, web_sys::IntersectionObserver)>,
    ),
    _listeners: Vec<Closure<dyn FnMut(Event)>>,
}

impl core::fmt::Debug for BrowserWindow {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BrowserWindow")
            .field("size", &self.surface.size())
            .finish_non_exhaustive()
    }
}

impl BrowserWindow {
    /// The registration this performs runs entirely on the page's
    /// objects; the returned future is `!Send` by design — wasm32 is
    /// single-threaded and every handle it holds is a JS object.
    ///
    /// # Panics
    ///
    /// When the page exposes no window, document, root or input element
    /// the host cannot function, so the constructor panics naming the
    /// missing object rather than failing the first paint later.
    ///
    /// # Errors
    ///
    /// [`BrowserGpuUnavailable`] when the browser provides no WebGPU
    /// adapter or device: the page cannot host a window at all, and the
    /// caller reports that to the page.
    #[expect(
        clippy::future_not_send,
        reason = "the future runs on the browser main thread via spawn_local; wasm32 is single-threaded so !Send state never crosses a thread"
    )]
    pub async fn new(
        schedule_frame: Rc<dyn Fn()>,
        occlusion_wake: Rc<dyn Fn()>,
    ) -> Result<Self, BrowserGpuUnavailable> {
        let browser_window =
            web_sys::window().expect("hydrolysis web platform: browser window unavailable");
        let document = browser_window
            .document()
            .expect("hydrolysis web platform: document unavailable");
        let root = find_or_create_root(&document);
        prepare_root(&root);
        let ime_input = find_or_create_ime_input(&document);
        let viewport = Viewport::new(&document);
        let safe_area = nami::binding(viewport.fit(&browser_window, &root));
        let pending_events = Rc::new(RefCell::new(Vec::new()));
        let redraw_requested = Rc::new(Cell::new(false));
        let scale_factor = Rc::new(Cell::new(browser_window.device_pixel_ratio()));
        let pending_resize = Rc::new(Cell::new(None));
        let touch_scroll_config = touch_scroll_config_for(&browser_window);

        let initial_resize = measure_root(&browser_window, &root);
        pending_resize.set(Some(initial_resize));
        pending_events.borrow_mut().push(InputEvent::Resize {
            width: initial_resize.width,
            height: initial_resize.height,
        });

        let surface =
            BrowserSurface::new(root.clone(), initial_resize.width, initial_resize.height).await?;

        let mut listeners = register_listeners(
            &root,
            &ime_input,
            pending_events.clone(),
            redraw_requested.clone(),
            schedule_frame.clone(),
        );
        // A rotation or a toolbar showing or hiding moves the viewport: the
        // root is refitted before it is measured, and the safe-area binding
        // re-lays the window out only when the insets actually changed.
        listeners.push(add_event_listener(browser_window.as_ref(), "resize", {
            let browser_window = browser_window.clone();
            let root = root.clone();
            let safe_area = safe_area.clone();
            let scale_factor = scale_factor.clone();
            let pending_resize = pending_resize.clone();
            let pending_events = pending_events.clone();
            let redraw_requested = redraw_requested.clone();
            let schedule_frame = schedule_frame.clone();
            move |_event| {
                let insets = viewport.fit(&browser_window, &root);
                if insets != safe_area.snapshot() {
                    safe_area.set(insets);
                }
                let resize = measure_root(&browser_window, &root);
                scale_factor.set(resize.scale_factor);
                pending_resize.set(Some(resize));
                pending_events.borrow_mut().push(InputEvent::Resize {
                    width: resize.width,
                    height: resize.height,
                });
                redraw_requested.set(true);
                schedule_frame();
            }
        }));
        // A hidden page's rAF callback never fires, so the wake also
        // pulls the occlusion report into the pump synchronously — the
        // hide must be learned here, or the pump could never log it.
        listeners.push(add_event_listener(document.as_ref(), "visibilitychange", {
            let occlusion_wake = occlusion_wake.clone();
            move |_event| occlusion_wake()
        }));
        // `document.hidden` covers a backgrounded tab; what it cannot see
        // is the page visible while its root is not — scrolled out of
        // view, or `display:none`'d by the app's own minimized state. The
        // IntersectionObserver is the public API that reports it.
        let offscreen = Rc::new(Cell::new(false));
        let intersection_observer = {
            let offscreen = offscreen.clone();
            let callback: Closure<dyn FnMut(js_sys::Array, web_sys::IntersectionObserver)> =
                Closure::wrap(Box::new(
                    move |entries: js_sys::Array, _observer: web_sys::IntersectionObserver| {
                        for entry in entries.iter() {
                            let entry =
                                entry.unchecked_into::<web_sys::IntersectionObserverEntry>();
                            offscreen.set(!entry.is_intersecting());
                        }
                        occlusion_wake();
                    },
                ));
            let observer = web_sys::IntersectionObserver::new(callback.as_ref().unchecked_ref())
                .expect("hydrolysis web platform: failed to create IntersectionObserver");
            observer.observe(&root);
            (observer, callback)
        };

        Ok(Self {
            host_window: browser_window,
            document,
            root,
            ime_input,
            surface,
            pending_events,
            redraw_requested,

            offscreen,
            scale_factor,
            pending_resize,
            safe_area,
            schedule_frame,
            current_cursor_style: CursorStyle::Arrow,
            touch_scroll_config,
            top_edge_color: None,
            _intersection_observer: intersection_observer,
            _listeners: listeners,
        })
    }

    /// Tells the page that the first frame is on screen, as a bubbling
    /// `waterui:first-frame` event: the page's launch screen listens for it and
    /// stands down.
    pub(crate) fn announce_first_frame(&self) {
        let init = web_sys::CustomEventInit::new();
        init.set_bubbles(true);
        let event = web_sys::CustomEvent::new_with_event_init_dict("waterui:first-frame", &init)
            .expect("hydrolysis web platform: failed to build the first-frame event");
        self.root
            .dispatch_event(&event)
            .expect("hydrolysis web platform: failed to dispatch the first-frame event");
    }

    /// The page's safe area: the binding the runner installs as the
    /// window's `WindowSafeArea`.
    #[must_use]
    pub fn safe_area(&self) -> nami::Binding<waterui_layout::padding::EdgeInsets> {
        self.safe_area.clone()
    }

    /// Consumes the pending redraw request, reporting whether one was set.
    #[must_use]
    pub fn take_redraw_request(&self) -> bool {
        self.redraw_requested.replace(false)
    }
}

impl PlatformWindow for BrowserWindow {
    fn content_size(&self) -> (u32, u32) {
        // The root's box at the device pixel ratio is exactly the drawable
        // content area: the engine's canvases fill it.
        self.surface.size()
    }

    fn apply_properties(&mut self, window: &WuiWindow) {
        self.document
            .set_title(window.display_title().snapshot().as_str());

        match window.state.snapshot() {
            WindowState::Normal => {
                self.root
                    .style()
                    .set_property("display", "block")
                    .expect("hydrolysis web platform: failed to show the root element");
            }
            WindowState::Minimized | WindowState::Closed => {
                self.root
                    .style()
                    .set_property("display", "none")
                    .expect("hydrolysis web platform: failed to hide the root element");
                return;
            }
            WindowState::Fullscreen => {
                panic!("hydrolysis web platform does not support fullscreen window state yet")
            }
            WindowState::Maximized => {
                panic!("hydrolysis web platform does not support maximized window state yet")
            }
        }

        if self.root.client_width() == 0 || self.root.client_height() == 0 {
            let frame = window.frame.snapshot();
            self.root
                .style()
                .set_property("width", &format!("{}px", frame.width().max(1.0)))
                .expect("hydrolysis web platform: failed to set the fallback root width");
            self.root
                .style()
                .set_property("height", &format!("{}px", frame.height().max(1.0)))
                .expect("hydrolysis web platform: failed to set the fallback root height");

            let resize = measure_root(&self.host_window, &self.root);
            self.pending_resize.set(Some(resize));
        }
    }

    fn drain_events(&mut self) -> Vec<InputEvent> {
        if let Some(resize) = self.pending_resize.take() {
            self.scale_factor.set(resize.scale_factor);
            self.surface.resize(resize.width, resize.height);
        }
        core::mem::take(&mut self.pending_events.borrow_mut())
    }

    /// The page's own report: `document.hidden` covers a backgrounded
    /// tab or window, and the `IntersectionObserver` cell covers the
    /// root scrolled out of view or `display:none`'d by an app-driven
    /// minimized state.
    fn is_occluded(&self) -> bool {
        self.document.hidden() || self.offscreen.get()
    }

    fn request_redraw(&self) {
        self.redraw_requested.set(true);
    }

    /// The page's `requestAnimationFrame` coalescer — already what every
    /// browser listener and executor wakeup calls for a frame; a repeated
    /// request while one is pending is merged by the runner's
    /// `raf_pending` flag, so frame counts are unchanged.
    fn frame_wake(&self) -> Rc<dyn Fn()> {
        Rc::clone(&self.schedule_frame)
    }

    fn scale_factor(&self) -> f64 {
        self.scale_factor.get()
    }

    /// The browser publishes no touch-gesture parameters, and CSS pixels
    /// are the window's density-1 logical units, so it reports the
    /// density-1 model of the system its user agent names —
    /// `UIScrollView`'s on Apple systems, Android's `OverScroller`
    /// elsewhere.
    fn touch_scroll_config(&self) -> Option<TouchScrollConfig> {
        Some(self.touch_scroll_config)
    }

    fn sync_text_input_state(&mut self, state: Option<TextInputState>) {
        if let Some(state) = state {
            self.ime_input.set_type(match state.purpose() {
                TextInputPurpose::Normal => "text",
                TextInputPurpose::Password => "password",
            });
            let style = self.ime_input.style();
            style
                .set_property("left", &format!("{}px", state.x()))
                .expect("hydrolysis web platform: failed to place IME input");
            style
                .set_property("top", &format!("{}px", state.y()))
                .expect("hydrolysis web platform: failed to place IME input");
            style
                .set_property("width", &format!("{}px", state.width().max(1.0)))
                .expect("hydrolysis web platform: failed to size IME input width");
            style
                .set_property("height", &format!("{}px", state.height().max(1.0)))
                .expect("hydrolysis web platform: failed to size IME input height");
            let _ = self.ime_input.focus();
        } else {
            self.ime_input.set_value("");
            let _ = self.ime_input.blur();
            let _ = self.root.focus();
        }
    }

    fn set_cursor_style(&mut self, style: CursorStyle) {
        if self.current_cursor_style == style {
            return;
        }
        self.current_cursor_style = style;
        self.root
            .style()
            .set_property("cursor", map_cursor_style(style))
            .expect("hydrolysis web platform: failed to update cursor style");
    }
}

impl GpuSurfaceWindow for BrowserWindow {
    type Presentation = BrowserSurface;
    fn surface(&mut self) -> &mut BrowserSurface {
        &mut self.surface
    }

    /// The page background is what a mobile browser shows behind its
    /// status bar: iOS Safari fills that band with the background of the
    /// page when no fixed header covers the top edge, and never draws page
    /// content there while the page is unscrolled. Matching the page
    /// background to the app's top edge makes the band continue the app.
    const SAMPLES_TOP_EDGE: bool = true;

    fn set_top_edge_color(&mut self, color: waterui_graphics::draw::WorkingColor) {
        if self.top_edge_color == Some(color) {
            return;
        }
        self.top_edge_color = Some(color);
        let [r, g, b, a] =
            peniko::color::AlphaColor::<waterui_graphics::draw::LinearDisplayP3>::new(
                color.components,
            )
            .convert::<peniko::color::DisplayP3>()
            .components;
        let css = format!("color(display-p3 {r} {g} {b} / {a})");
        let html = self
            .document
            .document_element()
            .expect("hydrolysis web platform: document has no root element")
            .dyn_into::<HtmlElement>()
            .expect("hydrolysis web platform: the document element is not an HTML element");
        let body = self
            .document
            .body()
            .expect("hydrolysis web platform: document body unavailable");
        for element in [&html, &body] {
            element
                .style()
                .set_property("background-color", &css)
                .expect("hydrolysis web platform: failed to set the page background");
        }
    }
}

/// The id of the page's root element, which the CLI's page template carries.
const ROOT_ID: &str = "waterui-root";

fn find_or_create_root(document: &Document) -> HtmlElement {
    if let Some(element) = document.get_element_by_id(ROOT_ID) {
        return element
            .dyn_into::<HtmlElement>()
            .expect("hydrolysis web platform: #waterui-root is not an HTML element");
    }

    let root = document
        .create_element("div")
        .expect("hydrolysis web platform: failed to create the root element")
        .dyn_into::<HtmlElement>()
        .expect("hydrolysis web platform: created node is not an HTML element");
    root.set_id(ROOT_ID);
    document
        .body()
        .expect("hydrolysis web platform: document body unavailable")
        .append_child(&root)
        .expect("hydrolysis web platform: failed to append the root element to body");
    root
}

/// Makes the page's root element the runtime's surface, whether the page
/// supplied it or the runtime created it. The engine's canvases inside it
/// take no pointer events, so input over engine content lands on the root
/// itself.
///
/// - Geometry: anchored to the page's top-left corner and as wide as the
///   layout viewport; [`Viewport::fit`] sets its height. It is positioned
///   absolutely, never `fixed`: a browser that composites page content
///   under its own toolbars clips a fixed element to the visible viewport,
///   while document content extends beneath them.
/// - `touch-action: none`: Hydrolysis recognizes every gesture itself, so
///   the browser must never claim a touch for panning or zooming; when it
///   does, it cancels the pointer (`pointercancel` instead of `pointerup`)
///   and a tap never reaches the view under it.
/// - `tabindex` 0: the root receives the keys, which a focusable element
///   alone can do.
/// - `outline: none`: Hydrolysis draws focus itself.
fn prepare_root(root: &HtmlElement) {
    root.set_tab_index(0);
    let style = root.style();
    for (property, value) in [
        ("display", "block"),
        ("position", "absolute"),
        ("left", "0"),
        ("top", "0"),
        ("width", "100%"),
        ("height", "100%"),
        ("touch-action", "none"),
        ("outline", "none"),
    ] {
        style
            .set_property(property, value)
            .expect("hydrolysis web platform: failed to style the root element");
    }
}

/// The page's viewport as the runtime lays the window out in it: a hidden
/// probe whose computed style resolves what has no script API —
/// `env(safe-area-inset-*)` as its padding, the large and small viewport
/// heights (`100lvh`, `100svh`) as its minimum and maximum height.
struct Viewport {
    probe: HtmlElement,
}

impl Viewport {
    fn new(document: &Document) -> Self {
        let probe = document
            .create_element("div")
            .expect("hydrolysis web platform: failed to create the viewport probe")
            .dyn_into::<HtmlElement>()
            .expect("hydrolysis web platform: created node is not an HTML element");
        probe
            .set_attribute("aria-hidden", "true")
            .expect("hydrolysis web platform: failed to hide the viewport probe");
        let style = probe.style();
        for (property, value) in [
            ("position", "fixed"),
            ("left", "0"),
            ("top", "0"),
            ("width", "0"),
            ("visibility", "hidden"),
            ("pointer-events", "none"),
            (
                "padding",
                "env(safe-area-inset-top) env(safe-area-inset-right) \
                 env(safe-area-inset-bottom) env(safe-area-inset-left)",
            ),
            ("min-height", "100lvh"),
            ("max-height", "100svh"),
        ] {
            style
                .set_property(property, value)
                .expect("hydrolysis web platform: failed to style the viewport probe");
        }
        document
            .body()
            .expect("hydrolysis web platform: document body unavailable")
            .append_child(&probe)
            .expect("hydrolysis web platform: failed to append the viewport probe");
        Self { probe }
    }

    /// Sizes `root` for the viewport as it is now and returns the window's
    /// safe area, in CSS pixels — the window's logical units.
    ///
    /// A browser whose toolbars retract over the page — the large viewport
    /// taller than the small one — composites page content beneath them:
    /// iOS Safari draws document content under its floating bottom toolbar.
    /// The root then reaches past the bottom of the visible viewport by the
    /// height of the browser's own chrome (`outerHeight − innerHeight`), so
    /// the app paints under the toolbar, and that extension joins the
    /// bottom inset, so its content still lays out inside the visible
    /// viewport. The browser reports only the chrome's total, not how it
    /// splits above and below the page, so the extension can run past the
    /// screen's bottom edge by the chrome above the page; that part is
    /// painted and never shown. A browser with fixed chrome — every desktop
    /// browser — reports equal viewports and gets no extension.
    fn fit(
        &self,
        browser_window: &BrowserHostWindow,
        root: &HtmlElement,
    ) -> waterui_layout::padding::EdgeInsets {
        let style = browser_window
            .get_computed_style(&self.probe)
            .expect("hydrolysis web platform: failed to compute the viewport probe style")
            .expect("hydrolysis web platform: the viewport probe has no computed style");
        let length = |property: &str| -> f64 {
            let value = style
                .get_property_value(property)
                .expect("hydrolysis web platform: failed to read a viewport probe length");
            value
                .strip_suffix("px")
                .and_then(|pixels| pixels.parse::<f64>().ok())
                .unwrap_or_else(|| {
                    panic!("hydrolysis web platform: viewport probe {property} resolved to {value:?}, not a pixel length")
                })
        };
        let extension = if length("min-height") > length("max-height") {
            let dimension = |value: Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>,
                             name: &str| {
                value
                    .unwrap_or_else(|_| panic!("hydrolysis web platform: failed to query {name}"))
                    .as_f64()
                    .unwrap_or_else(|| panic!("hydrolysis web platform: {name} is not numeric"))
            };
            (dimension(browser_window.outer_height(), "outerHeight")
                - dimension(browser_window.inner_height(), "innerHeight"))
            .max(0.0)
        } else {
            0.0
        };
        root.style()
            .set_property("height", &format!("calc(100% + {extension}px)"))
            .expect("hydrolysis web platform: failed to size the root element");
        let inset = |property: &str| crate::num_cast::f64_as_f32(length(property));
        waterui_layout::padding::EdgeInsets::new(
            inset("padding-top"),
            crate::num_cast::f64_as_f32(length("padding-bottom") + extension),
            inset("padding-left"),
            inset("padding-right"),
        )
    }
}

fn find_or_create_ime_input(document: &Document) -> HtmlInputElement {
    if let Some(element) = document.get_element_by_id("waterui-ime") {
        return element
            .dyn_into::<HtmlInputElement>()
            .expect("hydrolysis web platform: #waterui-ime is not an <input>");
    }

    let input = document
        .create_element("input")
        .expect("hydrolysis web platform: failed to create IME input")
        .dyn_into::<HtmlInputElement>()
        .expect("hydrolysis web platform: created node is not an input");
    input.set_id("waterui-ime");
    let style = input.style();
    style
        .set_property("position", "fixed")
        .expect("hydrolysis web platform: failed to style IME input");
    style
        .set_property("opacity", "0")
        .expect("hydrolysis web platform: failed to style IME input opacity");
    style
        .set_property("pointer-events", "none")
        .expect("hydrolysis web platform: failed to style IME input pointer-events");
    style
        .set_property("z-index", "-1")
        .expect("hydrolysis web platform: failed to style IME input z-index");
    style
        .set_property("left", "0px")
        .expect("hydrolysis web platform: failed to style IME input left");
    style
        .set_property("top", "0px")
        .expect("hydrolysis web platform: failed to style IME input top");
    style
        .set_property("width", "1px")
        .expect("hydrolysis web platform: failed to style IME input width");
    style
        .set_property("height", "1px")
        .expect("hydrolysis web platform: failed to style IME input height");
    document
        .body()
        .expect("hydrolysis web platform: document body unavailable")
        .append_child(&input)
        .expect("hydrolysis web platform: failed to append IME input to body");
    input
}

fn measure_root(browser_window: &BrowserHostWindow, root: &HtmlElement) -> PendingResize {
    let scale_factor = browser_window.device_pixel_ratio();
    assert!(
        scale_factor.is_finite() && scale_factor > 0.0,
        "hydrolysis web platform received invalid devicePixelRatio {scale_factor}"
    );

    let rect = root.get_bounding_client_rect();
    let mut logical_width = rect.width();
    let mut logical_height = rect.height();
    if logical_width <= 0.0 {
        logical_width = f64::from(root.client_width());
    }
    if logical_height <= 0.0 {
        logical_height = f64::from(root.client_height());
    }
    if logical_width <= 0.0 {
        logical_width = browser_window
            .inner_width()
            .expect("hydrolysis web platform: failed to query window width")
            .as_f64()
            .expect("hydrolysis web platform: window width is not numeric");
    }
    if logical_height <= 0.0 {
        logical_height = browser_window
            .inner_height()
            .expect("hydrolysis web platform: failed to query window height")
            .as_f64()
            .expect("hydrolysis web platform: window height is not numeric");
    }

    PendingResize {
        width: crate::num_cast::f64_as_u32((logical_width.max(1.0) * scale_factor).round()),
        height: crate::num_cast::f64_as_u32((logical_height.max(1.0) * scale_factor).round()),
        scale_factor,
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "the listener registration enumerates every DOM event hook once; the length is the enumeration, not logic"
)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the shared Rc state is moved into listener closures; borrowing would fight the closure 'static bounds"
)]
fn register_listeners(
    root: &HtmlElement,
    ime_input: &HtmlInputElement,
    pending_events: Rc<RefCell<Vec<InputEvent>>>,
    redraw_requested: Rc<Cell<bool>>,
    schedule_frame: Rc<dyn Fn()>,
) -> Vec<Closure<dyn FnMut(Event)>> {
    let mut listeners = Vec::new();
    let composing = Rc::new(Cell::new(false));
    let suppress_next_input = Rc::new(Cell::new(false));
    let root_target: EventTarget = <HtmlElement as AsRef<EventTarget>>::as_ref(root).clone();
    let ime_input_target: EventTarget =
        <HtmlInputElement as AsRef<EventTarget>>::as_ref(ime_input).clone();

    {
        let root = root.clone();
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let schedule_frame = schedule_frame.clone();
        listeners.push(add_event_listener(&root_target, "keydown", move |event| {
            if !targets_root(&event, &root) {
                return;
            }
            let event = event
                .dyn_into::<KeyboardEvent>()
                .expect("hydrolysis web platform: keydown event had unexpected type");
            if matches!(event.key().as_str(), "Tab" | "Enter" | " ") {
                event.prevent_default();
            }
            pending_events.borrow_mut().push(InputEvent::Key {
                key: map_keyboard_key(&event),
                logical_key: map_w3c_key(&event),
                physical_code: map_w3c_code(&event),
                repeat: event.repeat(),
                state: KeyState::Pressed,
                modifiers: map_modifiers_from_keyboard(&event),
            });
            redraw_requested.set(true);
            schedule_frame();
        }));
    }

    {
        let root = root.clone();
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let schedule_frame = schedule_frame.clone();
        listeners.push(add_event_listener(&root_target, "keyup", move |event| {
            if !targets_root(&event, &root) {
                return;
            }
            let event = event
                .dyn_into::<KeyboardEvent>()
                .expect("hydrolysis web platform: keyup event had unexpected type");
            if matches!(event.key().as_str(), "Tab" | "Enter" | " ") {
                event.prevent_default();
            }
            pending_events.borrow_mut().push(InputEvent::Key {
                key: map_keyboard_key(&event),
                logical_key: map_w3c_key(&event),
                physical_code: map_w3c_code(&event),
                repeat: event.repeat(),
                state: KeyState::Released,
                modifiers: map_modifiers_from_keyboard(&event),
            });
            redraw_requested.set(true);
            schedule_frame();
        }));
    }

    {
        let root = root.clone();
        let ime_input = ime_input.clone();
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let schedule_frame = schedule_frame.clone();
        listeners.push(add_event_listener(
            &root_target,
            "pointerdown",
            move |event| {
                if !targets_root(&event, &root) {
                    return;
                }
                let event = event
                    .dyn_into::<PointerEvent>()
                    .expect("hydrolysis web platform: pointerdown event had unexpected type");
                event.prevent_default();
                // Preventing the default also prevents the press from focusing
                // the root, which receives the keys. Text editing owns the
                // hidden input while it holds focus; anything else returns
                // focus to the root.
                let editing = root
                    .owner_document()
                    .and_then(|document| document.active_element())
                    .is_some_and(|active| active == *ime_input.as_ref());
                if !editing {
                    let _ = root.focus();
                }
                let (x, y) = event_position(
                    &root,
                    f64::from(event.client_x()),
                    f64::from(event.client_y()),
                );
                pending_events.borrow_mut().push(InputEvent::PointerDown {
                    id: pointer_id(&event),
                    kind: pointer_kind(&event),
                    x,
                    y,
                    button: map_pointer_button(event.button()),
                });
                redraw_requested.set(true);
                schedule_frame();
            },
        ));
    }

    {
        let root = root.clone();
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let schedule_frame = schedule_frame.clone();
        listeners.push(add_event_listener(
            &root_target,
            "pointerup",
            move |event| {
                if !targets_root(&event, &root) {
                    return;
                }
                let event = event
                    .dyn_into::<PointerEvent>()
                    .expect("hydrolysis web platform: pointerup event had unexpected type");
                event.prevent_default();
                let (x, y) = event_position(
                    &root,
                    f64::from(event.client_x()),
                    f64::from(event.client_y()),
                );
                pending_events.borrow_mut().push(InputEvent::PointerUp {
                    id: pointer_id(&event),
                    kind: pointer_kind(&event),
                    x,
                    y,
                    button: map_pointer_button(event.button()),
                });
                redraw_requested.set(true);
                schedule_frame();
            },
        ));
    }

    {
        let root = root.clone();
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let schedule_frame = schedule_frame.clone();
        listeners.push(add_event_listener(
            &root_target,
            "pointermove",
            move |event| {
                if !targets_root(&event, &root) {
                    return;
                }
                let event = event
                    .dyn_into::<PointerEvent>()
                    .expect("hydrolysis web platform: pointermove event had unexpected type");
                let (x, y) = event_position(
                    &root,
                    f64::from(event.client_x()),
                    f64::from(event.client_y()),
                );
                pending_events.borrow_mut().push(InputEvent::PointerMove {
                    id: pointer_id(&event),
                    kind: pointer_kind(&event),
                    x,
                    y,
                });
                redraw_requested.set(true);
                schedule_frame();
            },
        ));
    }

    {
        let root = root.clone();
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let schedule_frame = schedule_frame.clone();
        listeners.push(add_event_listener(
            &root_target,
            "pointercancel",
            move |event| {
                if !targets_root(&event, &root) {
                    return;
                }
                let event = event
                    .dyn_into::<PointerEvent>()
                    .expect("hydrolysis web platform: pointercancel event had unexpected type");
                pending_events.borrow_mut().push(InputEvent::PointerCancel {
                    id: pointer_id(&event),
                    kind: pointer_kind(&event),
                });
                redraw_requested.set(true);
                schedule_frame();
            },
        ));
    }

    {
        let root = root.clone();
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let schedule_frame = schedule_frame.clone();
        listeners.push(add_event_listener(&root_target, "wheel", move |event| {
            if !targets_root(&event, &root) {
                return;
            }
            let event = event
                .dyn_into::<WheelEvent>()
                .expect("hydrolysis web platform: wheel event had unexpected type");
            event.prevent_default();
            let (x, y) = event_position(
                &root,
                f64::from(event.client_x()),
                f64::from(event.client_y()),
            );
            pending_events.borrow_mut().push(InputEvent::Scroll {
                x,
                y,
                // DOM WheelEvent deltas are positive right/down; Hydrolysis input
                // takes winit's opposite sign.
                dx: -crate::num_cast::f64_as_f32(event.delta_x()),
                dy: -crate::num_cast::f64_as_f32(event.delta_y()),
                is_line_delta: event.delta_mode() != WheelEvent::DOM_DELTA_PIXEL,
            });
            redraw_requested.set(true);
            schedule_frame();
        }));
    }

    {
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let schedule_frame = schedule_frame.clone();
        listeners.push(add_event_listener(
            ime_input.as_ref(),
            "keydown",
            move |event| {
                let event = event
                    .dyn_into::<KeyboardEvent>()
                    .expect("hydrolysis web platform: keydown event had unexpected type");
                let modifiers = map_modifiers_from_keyboard(&event);
                match event.key().as_str() {
                    "Backspace" => {
                        event.prevent_default();
                        pending_events.borrow_mut().push(InputEvent::Key {
                            key: KeyCode::Named("Backspace".to_string()),
                            logical_key: keyboard_types::Key::Named(
                                keyboard_types::NamedKey::Backspace,
                            ),
                            physical_code: keyboard_types::Code::Backspace,
                            repeat: event.repeat(),
                            state: KeyState::Pressed,
                            modifiers,
                        });
                    }
                    "Enter" => {
                        event.prevent_default();
                        pending_events.borrow_mut().push(InputEvent::ImeCommit {
                            text: "\n".to_string(),
                        });
                    }
                    "Tab" => {
                        event.prevent_default();
                        pending_events.borrow_mut().push(InputEvent::Key {
                            key: KeyCode::Named("Tab".to_string()),
                            logical_key: keyboard_types::Key::Named(keyboard_types::NamedKey::Tab),
                            physical_code: keyboard_types::Code::Tab,
                            repeat: event.repeat(),
                            state: KeyState::Pressed,
                            modifiers,
                        });
                    }
                    _ => return,
                }
                redraw_requested.set(true);
                schedule_frame();
            },
        ));
    }

    {
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let schedule_frame = schedule_frame.clone();
        let ime_input = ime_input.clone();
        let composing = composing.clone();
        let suppress_next_input = suppress_next_input.clone();
        listeners.push(add_event_listener(
            &ime_input_target,
            "input",
            move |event| {
                event.prevent_default();
                if suppress_next_input.replace(false) {
                    ime_input.set_value("");
                    return;
                }
                if composing.get() {
                    return;
                }
                let value = ime_input.value();
                if value.is_empty() {
                    return;
                }
                ime_input.set_value("");
                pending_events
                    .borrow_mut()
                    .push(InputEvent::ImeCommit { text: value });
                redraw_requested.set(true);
                schedule_frame();
            },
        ));
    }

    {
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let schedule_frame = schedule_frame.clone();
        let composing = composing.clone();
        listeners.push(add_event_listener(
            &ime_input_target,
            "compositionstart",
            move |event| {
                let event = event
                    .dyn_into::<CompositionEvent>()
                    .expect("hydrolysis web platform: compositionstart event had unexpected type");
                composing.set(true);
                pending_events.borrow_mut().push(InputEvent::ImePreedit {
                    text: composition_text(event),
                    // The DOM composition events report no caret offset.
                    caret: None,
                });
                redraw_requested.set(true);
                schedule_frame();
            },
        ));
    }

    {
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let schedule_frame = schedule_frame.clone();
        listeners.push(add_event_listener(
            &ime_input_target,
            "compositionupdate",
            move |event| {
                let event = event
                    .dyn_into::<CompositionEvent>()
                    .expect("hydrolysis web platform: compositionupdate event had unexpected type");
                pending_events.borrow_mut().push(InputEvent::ImePreedit {
                    text: composition_text(event),
                    // The DOM composition events report no caret offset.
                    caret: None,
                });
                redraw_requested.set(true);
                schedule_frame();
            },
        ));
    }

    {
        let ime_input = ime_input.clone();
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let schedule_frame = schedule_frame.clone();
        // `composing`/`suppress_next_input` end here — the originals move
        // into the closure rather than spending an Rc clone each.
        listeners.push(add_event_listener(
            &ime_input_target,
            "compositionend",
            move |event| {
                let event = event
                    .dyn_into::<CompositionEvent>()
                    .expect("hydrolysis web platform: compositionend event had unexpected type");
                composing.set(false);
                suppress_next_input.set(true);
                ime_input.set_value("");
                pending_events.borrow_mut().push(InputEvent::ImeCommit {
                    text: composition_text(event),
                });
                redraw_requested.set(true);
                schedule_frame();
            },
        ));
    }

    {
        // Both originals move into the blur closure — its registration is
        // the last use either Rc has.
        let schedule_frame = schedule_frame.clone();
        listeners.push(add_event_listener(
            &ime_input_target,
            "blur",
            move |_event| {
                pending_events.borrow_mut().push(InputEvent::ImeDisabled);
                redraw_requested.set(true);
                schedule_frame();
            },
        ));
    }

    listeners
}

// web_sys hands ownership of the event; only the borrowed payload is read
#[expect(
    clippy::needless_pass_by_value,
    reason = "the web-sys event is delivered by value and only its data string is read"
)]
fn composition_text(event: CompositionEvent) -> String {
    event.data().unwrap_or_default()
}

pub fn add_event_listener(
    target: &web_sys::EventTarget,
    event_name: &str,
    handler: impl 'static + FnMut(Event),
) -> Closure<dyn FnMut(Event)> {
    let closure = Closure::wrap(Box::new(handler) as Box<dyn FnMut(Event)>);
    target
        .add_event_listener_with_callback(event_name, closure.as_ref().unchecked_ref())
        .unwrap_or_else(|_| {
            panic!("hydrolysis web platform: failed to register {event_name} listener")
        });
    closure
}

#[cfg(any(feature = "video", feature = "webview-system"))]
/// The pointer and wheel events a hosted element yields to Hydrolysis
/// content painted above it.
const REDIRECTED_INPUT: [&str; 8] = [
    "pointerdown",
    "pointerup",
    "pointermove",
    "pointercancel",
    "wheel",
    "click",
    "dblclick",
    "contextmenu",
];

#[cfg(feature = "video")]
/// Makes `element`, an element the engine hosts in the page with no wheel
/// behaviour of its own — a `<video>` — yield every wheel event to the root
/// element, so a scroll view keeps scrolling under the pointer wherever the
/// element sits.
///
/// A capture listener on the element stops the event and dispatches a copy
/// to the root, whose listeners deliver it to Hydrolysis's hit testing as if
/// the element were not there. The listener stays registered while it is
/// held.
pub fn yield_wheel(element: &HtmlElement) -> Closure<dyn FnMut(Event)> {
    let closure = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
        let wheel = event
            .dyn_ref::<WheelEvent>()
            .expect("hydrolysis web platform: a wheel listener received another event");
        event.stop_immediate_propagation();
        event.prevent_default();
        page_root()
            .dispatch_event(&wheel_event_copy(wheel))
            .expect("hydrolysis web platform: failed to yield a wheel event");
    });
    element
        .add_event_listener_with_callback_and_bool("wheel", closure.as_ref().unchecked_ref(), true)
        .expect("hydrolysis web platform: failed to register the wheel yield");
    closure
}

#[cfg(any(feature = "video", feature = "webview-system"))]
/// A dispatchable copy of `event`, carrying everything the root's pointer
/// listeners read.
fn pointer_event_copy(event: &PointerEvent) -> PointerEvent {
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_client_x(event.client_x());
    init.set_client_y(event.client_y());
    init.set_screen_x(event.screen_x());
    init.set_screen_y(event.screen_y());
    init.set_button(event.button());
    init.set_buttons(event.buttons());
    init.set_alt_key(event.alt_key());
    init.set_ctrl_key(event.ctrl_key());
    init.set_meta_key(event.meta_key());
    init.set_shift_key(event.shift_key());
    init.set_pointer_id(event.pointer_id());
    init.set_pointer_type(&event.pointer_type());
    init.set_is_primary(event.is_primary());
    init.set_pressure(event.pressure());
    init.set_width(event.width());
    init.set_height(event.height());
    PointerEvent::new_with_event_init_dict(&event.type_(), &init)
        .expect("hydrolysis web platform: failed to copy a pointer event")
}

#[cfg(any(feature = "video", feature = "webview-system"))]
/// A dispatchable copy of `event`, carrying everything the root's wheel
/// listener reads.
fn wheel_event_copy(event: &WheelEvent) -> WheelEvent {
    let init = web_sys::WheelEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_client_x(event.client_x());
    init.set_client_y(event.client_y());
    init.set_alt_key(event.alt_key());
    init.set_ctrl_key(event.ctrl_key());
    init.set_meta_key(event.meta_key());
    init.set_shift_key(event.shift_key());
    init.set_delta_x(event.delta_x());
    init.set_delta_y(event.delta_y());
    init.set_delta_z(event.delta_z());
    init.set_delta_mode(event.delta_mode());
    WheelEvent::new_with_event_init_dict(&event.type_(), &init)
        .expect("hydrolysis web platform: failed to copy a wheel event")
}

/// Hit-taking surfaces over the rects where interactive Hydrolysis content
/// is painted above an element that keeps its input to itself — a
/// cross-origin `<iframe>`, whose events never reach the page, or a
/// `<video>`, whose native controls take input before any page listener.
///
/// Each occluded rect gets a transparent `<div>` in the root element, stacked
/// above the engine's stacking root and so above every plane. A press on it
/// is stopped and a copy of the pointer or wheel event is dispatched to the
/// root, whose listeners deliver it to Hydrolysis's hit testing. The shields
/// follow the occlusion as it changes and leave the page when this drops.
#[cfg(any(feature = "video", feature = "webview-system"))]
pub struct OcclusionShields {
    shields: Rc<RefCell<Vec<HtmlElement>>>,
    _watch: waterui_watcher_set::WatcherGuard,
    _listeners: ShieldListeners,
}

/// The listeners every shield shares, by event name.
#[cfg(any(feature = "video", feature = "webview-system"))]
type ShieldListeners = Rc<Vec<(&'static str, Closure<dyn FnMut(Event)>)>>;

#[cfg(any(feature = "video", feature = "webview-system"))]
impl OcclusionShields {
    /// Keeps shields over `occlusion`'s rects.
    pub fn new(occlusion: &crate::HostedOcclusion) -> Self {
        let listeners: ShieldListeners = Rc::new(
            REDIRECTED_INPUT
                .into_iter()
                .map(|name| {
                    let closure = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
                        event.stop_immediate_propagation();
                        event.prevent_default();
                        let copy: Option<Event> = event.dyn_ref::<PointerEvent>().map_or_else(
                            || {
                                event
                                    .dyn_ref::<WheelEvent>()
                                    .map(|wheel| wheel_event_copy(wheel).into())
                            },
                            |pointer| Some(pointer_event_copy(pointer).into()),
                        );
                        if let Some(copy) = copy {
                            page_root().dispatch_event(&copy).expect(
                                "hydrolysis web platform: failed to redirect shielded input",
                            );
                        }
                    });
                    (name, closure)
                })
                .collect(),
        );
        let shields = Rc::new(RefCell::new(Vec::new()));
        let sync = {
            let shields = Rc::clone(&shields);
            let listeners = Rc::clone(&listeners);
            move |rects: Vec<kurbo::Rect>| sync_shields(&shields, &listeners, &rects)
        };
        sync(occlusion.rects());
        let watch = occlusion.watch(sync);
        Self {
            shields,
            _watch: watch,
            _listeners: listeners,
        }
    }
}

#[cfg(any(feature = "video", feature = "webview-system"))]
impl Drop for OcclusionShields {
    fn drop(&mut self) {
        for shield in self.shields.take() {
            shield.remove();
        }
    }
}

/// Makes `shields` exactly one transparent `<div>` per rect, placed over it.
#[cfg(any(feature = "video", feature = "webview-system"))]
fn sync_shields(
    shields: &RefCell<Vec<HtmlElement>>,
    listeners: &ShieldListeners,
    rects: &[kurbo::Rect],
) {
    let mut shields = shields.borrow_mut();
    while shields.len() > rects.len() {
        shields.pop().expect("a surplus shield exists").remove();
    }
    while shields.len() < rects.len() {
        let root = page_root();
        let shield: HtmlElement = root
            .owner_document()
            .expect("hydrolysis web platform: the root element left its document")
            .create_element("div")
            .expect("hydrolysis web platform: failed to create an occlusion shield")
            .unchecked_into();
        // Above the engine's stacking root whatever the document order: a
        // shield can be created before the first frame appends that root.
        for (property, value) in [
            ("position", "absolute"),
            ("z-index", "1"),
            ("pointer-events", "auto"),
        ] {
            shield
                .style()
                .set_property(property, value)
                .expect("hydrolysis web platform: failed to style an occlusion shield");
        }
        for (name, closure) in listeners.iter() {
            shield
                .add_event_listener_with_callback(name, closure.as_ref().unchecked_ref())
                .unwrap_or_else(|_| {
                    panic!("hydrolysis web platform: failed to register the {name} shield")
                });
        }
        root.append_child(&shield)
            .expect("hydrolysis web platform: failed to add an occlusion shield");
        shields.push(shield);
    }
    for (shield, rect) in shields.iter().zip(rects) {
        let style = shield.style();
        for (property, value) in [
            ("left", rect.x0),
            ("top", rect.y0),
            ("width", rect.width()),
            ("height", rect.height()),
        ] {
            style
                .set_property(property, &format!("{value}px"))
                .expect("hydrolysis web platform: failed to place an occlusion shield");
        }
    }
}

/// The page's root element.
#[cfg(any(feature = "video", feature = "webview-system"))]
fn page_root() -> HtmlElement {
    web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(ROOT_ID))
        .expect("hydrolysis web platform: the page has no #waterui-root")
        .unchecked_into()
}

/// Whether `event` was dispatched to the root element itself. The engine's
/// canvases take no pointer events, so input over engine content targets
/// the root; an element the engine hosts takes its own input, which bubbles
/// up through the root and is not the engine's to handle.
fn targets_root(event: &Event, root: &HtmlElement) -> bool {
    event
        .target()
        .is_some_and(|target| target == *<HtmlElement as AsRef<EventTarget>>::as_ref(root))
}

fn event_position(root: &HtmlElement, client_x: f64, client_y: f64) -> (f32, f32) {
    let rect = root.get_bounding_client_rect();
    (
        crate::num_cast::f64_as_f32(client_x - rect.left()),
        crate::num_cast::f64_as_f32(client_y - rect.top()),
    )
}

fn map_modifiers_from_keyboard(event: &KeyboardEvent) -> Modifiers {
    Modifiers {
        shift: event.shift_key(),
        control: event.ctrl_key(),
        alt: event.alt_key(),
        super_key: event.meta_key(),
    }
}

/// The DOM `key` attribute already *is* the W3C UI Events logical key.
fn map_w3c_key(event: &KeyboardEvent) -> keyboard_types::Key {
    event.key().parse().unwrap_or(keyboard_types::Key::Named(
        keyboard_types::NamedKey::Unidentified,
    ))
}

/// The DOM `code` attribute already *is* the W3C UI Events physical code.
fn map_w3c_code(event: &KeyboardEvent) -> keyboard_types::Code {
    event
        .code()
        .parse()
        .unwrap_or(keyboard_types::Code::Unidentified)
}

fn map_keyboard_key(event: &KeyboardEvent) -> KeyCode {
    let key = event.key();
    if key == "Unidentified" {
        KeyCode::Unidentified
    } else if key.chars().count() == 1 {
        KeyCode::Character(key)
    } else {
        KeyCode::Named(key)
    }
}

const fn map_pointer_button(button: i16) -> PointerButton {
    match button {
        0 => PointerButton::Primary,
        1 => PointerButton::Middle,
        2 => PointerButton::Secondary,
        3 => PointerButton::Back,
        4 => PointerButton::Forward,
        other if other >= 0 => PointerButton::Other(crate::num_cast::i16_as_u16(other)),
        _ => PointerButton::Other(u16::MAX),
    }
}

fn pointer_id(event: &PointerEvent) -> u64 {
    u64::try_from(event.pointer_id())
        .expect("hydrolysis web platform: pointer id must be non-negative")
}

fn pointer_kind(event: &PointerEvent) -> PointerKind {
    match event.pointer_type().as_str() {
        "mouse" => PointerKind::Mouse,
        "touch" => PointerKind::Touch,
        "pen" => PointerKind::Pen,
        kind => panic!("hydrolysis web platform: unsupported pointer type {kind:?}"),
    }
}

fn map_cursor_style(style: CursorStyle) -> &'static str {
    match style {
        CursorStyle::Arrow => "default",
        CursorStyle::PointingHand => "pointer",
        CursorStyle::IBeam => "text",
        CursorStyle::Crosshair => "crosshair",
        CursorStyle::OpenHand => "grab",
        CursorStyle::ClosedHand => "grabbing",
        CursorStyle::NotAllowed => "not-allowed",
        CursorStyle::ResizeLeft => "w-resize",
        CursorStyle::ResizeRight => "e-resize",
        CursorStyle::ResizeUp => "n-resize",
        CursorStyle::ResizeDown => "s-resize",
        CursorStyle::ResizeLeftRight => "ew-resize",
        CursorStyle::ResizeUpDown => "ns-resize",
        CursorStyle::Move => "move",
        CursorStyle::Wait => "wait",
        CursorStyle::Copy => "copy",
        _ => panic!("unsupported CursorStyle variant in hydrolysis web backend"),
    }
}

pub use BrowserSurface as ExportedBrowserSurface;
pub use BrowserWindow as ExportedBrowserWindow;
