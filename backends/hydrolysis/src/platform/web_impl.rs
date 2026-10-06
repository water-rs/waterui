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
    CompositionEvent, Document, Event, EventTarget, HtmlCanvasElement, HtmlElement,
    HtmlInputElement, KeyboardEvent, PointerEvent, WheelEvent, Window as BrowserHostWindow,
};

use super::{
    CursorStyle, GpuSurfaceWindow, InputEvent, KeyCode, KeyState, Modifiers, PlatformWindow,
    PointerButton, PointerKind, SurfaceError, SurfaceFrame, SurfaceProvider, TextInputPurpose,
    TextInputState, WindowState, WuiWindow,
};

#[derive(Clone, Copy)]
struct PendingResize {
    width: u32,
    height: u32,
    scale_factor: f64,
}

pub struct BrowserSurface {
    instance: wgpu::Instance,
    /// Identity of this device creation chain for the engine pool.
    context_id: u64,
    surface: wgpu::Surface<'static>,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// Reports this device lost; taken when the device was opened.
    device_loss: crate::platform::DeviceLoss,
    config: wgpu::SurfaceConfiguration,
    /// The encoding of the negotiated canvas colour space, which the present
    /// pass writes: a WebGPU canvas interprets its values in its configured
    /// `colorSpace`, so the format alone does not say what to write.
    output_color: cherenkov_gpu::interop::OutputColor,
}

impl core::fmt::Debug for BrowserSurface {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BrowserSurface")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl BrowserSurface {
    #[expect(
        clippy::future_not_send,
        reason = "the future runs on the browser main thread via spawn_local; wasm32 is single-threaded so !Send state never crosses a thread"
    )]
    pub async fn new(canvas: HtmlCanvasElement, width: u32, height: u32) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
            .expect("hydrolysis web surface: failed to create canvas surface");
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .expect(
                "hydrolysis web surface: browser WebGPU adapter unavailable; ensure WebGPU is enabled",
            );

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("hydrolysis-web-device"),
                ..Default::default()
            })
            .await
            .expect("hydrolysis web surface: failed to request WebGPU device");
        let context_id = super::next_gpu_context_id();
        let shared_device = cherenkov_gpu::interop::SharedDevice {
            instance: instance.clone(),
            adapter: adapter.clone(),
            device: device.clone(),
            queue: queue.clone(),
        };
        let device_loss = crate::platform::DeviceLoss::observe(shared_device, context_id);

        // The canvas's (format, colour space) pair comes from the engine's
        // output negotiation, so the configured `colorSpace` and the encoding
        // the present pass writes always agree. The page asks for SDR: an
        // extended-range canvas puts Apple displays into EDR mode, which dims
        // every screenshot of the page and draws more power, and no
        // Hydrolysis host presents HDR. Display P3 keeps the wide gamut where
        // the browser offers it; sRGB is the space every canvas offers.
        let caps = surface.get_capabilities(&adapter);
        let selection = [wgpu::SurfaceColorSpace::DisplayP3, wgpu::SurfaceColorSpace::Srgb]
            .into_iter()
            .find_map(|color_space| {
                cherenkov_gpu::interop::select_output(
                    &caps,
                    wgpu::Backend::BrowserWebGpu,
                    cherenkov_gpu::interop::OutputRequest {
                        transparent: false,
                        color_space: Some(color_space),
                        sync: cherenkov_gpu::DisplaySync::Synchronized,
                    },
                )
                .ok()
            })
            .unwrap_or_else(|| {
                panic!(
                    "hydrolysis web surface: the canvas offers neither Display P3 nor sRGB output: {caps:?}"
                )
            });
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: selection.format,
            color_space: selection.color_space,
            width: width.max(1),
            height: height.max(1),
            present_mode: selection.present_mode,
            alpha_mode: selection.alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        Self {
            instance,
            context_id,
            surface,
            adapter,
            device,
            queue,
            device_loss,
            config,
            output_color: selection.output_color(),
        }
    }
}

impl SurfaceProvider for BrowserSurface {
    fn adapter(&self) -> &wgpu::Adapter {
        &self.adapter
    }

    fn device(&self) -> &wgpu::Device {
        &self.device
    }

    fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    fn device_loss(&self) -> &crate::platform::DeviceLoss {
        &self.device_loss
    }

    fn acquire(&mut self) -> Result<SurfaceFrame, SurfaceError> {
        let output = super::acquire_surface_texture(&self.surface)?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        Ok(SurfaceFrame::Browser { output, view })
    }

    fn present(&mut self, frame: SurfaceFrame) {
        match frame {
            SurfaceFrame::Browser { output, .. } => self.queue.present(output),
            SurfaceFrame::Offscreen { .. } => {
                panic!("hydrolysis web surface received an offscreen frame")
            }
            #[cfg(hydrolysis_winit)]
            SurfaceFrame::Window { .. } => {
                panic!("hydrolysis web surface received a native window frame")
            }
        }
    }

    fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    fn gpu_context_id(&self) -> u64 {
        self.context_id
    }

    fn output_color(&self) -> cherenkov_gpu::interop::OutputColor {
        self.output_color
    }

    fn output_alpha(&self) -> cherenkov_gpu::interop::OutputAlpha {
        cherenkov_gpu::interop::surface_output_alpha(self.config.alpha_mode)
    }

    fn shared_device(&self) -> cherenkov_gpu::interop::SharedDevice {
        cherenkov_gpu::interop::SharedDevice {
            instance: self.instance.clone(),
            adapter: self.adapter.clone(),
            device: self.device.clone(),
            queue: self.queue.clone(),
        }
    }
}

/// The browser window: the page's canvas element plus the document, input and
/// pointer plumbing attached to it.
pub struct BrowserWindow {
    host_window: BrowserHostWindow,
    document: Document,
    canvas: HtmlCanvasElement,
    ime_input: HtmlInputElement,
    surface: BrowserSurface,
    pending_events: Rc<RefCell<Vec<InputEvent>>>,
    redraw_requested: Rc<Cell<bool>>,
    /// The canvas's `IntersectionObserver` report: `true` while the
    /// browser counts it off-screen — scrolled out of view, or
    /// `display:none`'d by an app-driven minimized window state.
    offscreen: Rc<Cell<bool>>,
    scale_factor: Rc<Cell<f64>>,
    pending_resize: Rc<Cell<Option<PendingResize>>>,
    /// The page's safe area, which the window's `WindowSafeArea` installs;
    /// re-read from the probe on every resize.
    safe_area: nami::Binding<waterui_layout::padding::EdgeInsets>,
    current_cursor_style: CursorStyle,
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
            .field("canvas_width", &self.canvas.width())
            .field("canvas_height", &self.canvas.height())
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
    /// When the page exposes no window, document, canvas or input element
    /// the host cannot function, so the constructor panics naming the
    /// missing object rather than failing the first paint later.
    #[expect(
        clippy::future_not_send,
        reason = "the future runs on the browser main thread via spawn_local; wasm32 is single-threaded so !Send state never crosses a thread"
    )]
    pub async fn new(schedule_frame: Rc<dyn Fn()>, occlusion_wake: Rc<dyn Fn()>) -> Self {
        let browser_window =
            web_sys::window().expect("hydrolysis web platform: browser window unavailable");
        let document = browser_window
            .document()
            .expect("hydrolysis web platform: document unavailable");
        let canvas = find_or_create_canvas(&document);
        prepare_canvas(&canvas);
        let ime_input = find_or_create_ime_input(&document);
        let safe_area_probe = create_safe_area_probe(&document);
        let safe_area = nami::binding(read_safe_area(&browser_window, &safe_area_probe));
        let pending_events = Rc::new(RefCell::new(Vec::new()));
        let redraw_requested = Rc::new(Cell::new(false));
        let scale_factor = Rc::new(Cell::new(browser_window.device_pixel_ratio()));
        let pending_resize = Rc::new(Cell::new(None));

        let initial_resize = measure_canvas(&browser_window, &canvas);
        apply_canvas_resize(&canvas, initial_resize);
        pending_resize.set(Some(initial_resize));
        pending_events.borrow_mut().push(InputEvent::Resize {
            width: initial_resize.width,
            height: initial_resize.height,
        });

        let surface =
            BrowserSurface::new(canvas.clone(), initial_resize.width, initial_resize.height).await;

        let mut listeners = register_listeners(
            &browser_window,
            &canvas,
            &ime_input,
            pending_events.clone(),
            redraw_requested.clone(),
            scale_factor.clone(),
            pending_resize.clone(),
            schedule_frame.clone(),
        );
        // A hidden page's rAF callback never fires, so the wake also
        // pulls the occlusion report into the pump synchronously — the
        // hide must be learned here, or the pump could never log it.
        // A rotation or a toolbar showing or hiding moves the insets; the
        // binding re-lays the window out only when they actually changed.
        listeners.push(add_event_listener(browser_window.as_ref(), "resize", {
            let browser_window = browser_window.clone();
            let safe_area = safe_area.clone();
            move |_event| {
                let insets = read_safe_area(&browser_window, &safe_area_probe);
                if insets != safe_area.snapshot() {
                    safe_area.set(insets);
                }
            }
        }));
        listeners.push(add_event_listener(document.as_ref(), "visibilitychange", {
            let occlusion_wake = occlusion_wake.clone();
            move |_event| occlusion_wake()
        }));
        // `document.hidden` covers a backgrounded tab; what it cannot see
        // is the page visible while its canvas is not — scrolled out of
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
            observer.observe(&canvas);
            (observer, callback)
        };

        Self {
            host_window: browser_window,
            document,
            canvas,
            ime_input,
            surface,
            pending_events,
            redraw_requested,

            offscreen,
            scale_factor,
            pending_resize,
            safe_area,
            current_cursor_style: CursorStyle::Arrow,
            _intersection_observer: intersection_observer,
            _listeners: listeners,
        }
    }

    /// Tells the page that the first frame is on the canvas, as a bubbling
    /// `waterui:first-frame` event: the page's launch screen listens for it and
    /// stands down.
    pub(crate) fn announce_first_frame(&self) {
        let init = web_sys::CustomEventInit::new();
        init.set_bubbles(true);
        let event = web_sys::CustomEvent::new_with_event_init_dict("waterui:first-frame", &init)
            .expect("hydrolysis web platform: failed to build the first-frame event");
        self.canvas
            .dispatch_event(&event)
            .expect("hydrolysis web platform: failed to dispatch the first-frame event");
    }

    /// The page's safe area: the binding the runner installs as the
    /// window's `WindowSafeArea`.
    pub fn safe_area(&self) -> nami::Binding<waterui_layout::padding::EdgeInsets> {
        self.safe_area.clone()
    }

    /// Consumes the pending redraw request, reporting whether one was set.
    pub fn take_redraw_request(&self) -> bool {
        self.redraw_requested.replace(false)
    }
}

impl PlatformWindow for BrowserWindow {
    fn content_size(&self) -> (u32, u32) {
        // The canvas's backing store is exactly the drawable content area.
        self.surface.size()
    }

    fn apply_properties(&mut self, window: &WuiWindow) {
        self.document
            .set_title(window.display_title().snapshot().as_str());

        match window.state.snapshot() {
            WindowState::Normal => {
                self.canvas
                    .style()
                    .set_property("display", "block")
                    .expect("hydrolysis web platform: failed to show canvas");
            }
            WindowState::Minimized | WindowState::Closed => {
                self.canvas
                    .style()
                    .set_property("display", "none")
                    .expect("hydrolysis web platform: failed to hide canvas");
                return;
            }
            WindowState::Fullscreen => {
                panic!("hydrolysis web platform does not support fullscreen window state yet")
            }
            WindowState::Maximized => {
                panic!("hydrolysis web platform does not support maximized window state yet")
            }
        }

        if self.canvas.client_width() == 0 || self.canvas.client_height() == 0 {
            let frame = window.frame.snapshot();
            self.canvas
                .style()
                .set_property("width", &format!("{}px", frame.width().max(1.0)))
                .expect("hydrolysis web platform: failed to set fallback canvas width");
            self.canvas
                .style()
                .set_property("height", &format!("{}px", frame.height().max(1.0)))
                .expect("hydrolysis web platform: failed to set fallback canvas height");

            let resize = measure_canvas(&self.host_window, &self.canvas);
            apply_canvas_resize(&self.canvas, resize);
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
    /// canvas scrolled out of view or `display:none`'d by an app-driven
    /// minimized state.
    fn is_occluded(&self) -> bool {
        self.document.hidden() || self.offscreen.get()
    }

    fn request_redraw(&self) {
        self.redraw_requested.set(true);
    }

    fn scale_factor(&self) -> f64 {
        self.scale_factor.get()
    }

    fn sync_text_input_state(&mut self, state: Option<TextInputState>) {
        if let Some(state) = state {
            self.ime_input.set_type(match state.purpose {
                TextInputPurpose::Normal => "text",
                TextInputPurpose::Password => "password",
            });
            let style = self.ime_input.style();
            style
                .set_property("left", &format!("{}px", state.x))
                .expect("hydrolysis web platform: failed to place IME input");
            style
                .set_property("top", &format!("{}px", state.y))
                .expect("hydrolysis web platform: failed to place IME input");
            style
                .set_property("width", &format!("{}px", state.width.max(1.0)))
                .expect("hydrolysis web platform: failed to size IME input width");
            style
                .set_property("height", &format!("{}px", state.height.max(1.0)))
                .expect("hydrolysis web platform: failed to size IME input height");
            let _ = self.ime_input.focus();
        } else {
            self.ime_input.set_value("");
            let _ = self.ime_input.blur();
            let _ = self.canvas.focus();
        }
    }

    fn set_cursor_style(&mut self, style: CursorStyle) {
        if self.current_cursor_style == style {
            return;
        }
        self.current_cursor_style = style;
        self.canvas
            .style()
            .set_property("cursor", map_cursor_style(style))
            .expect("hydrolysis web platform: failed to update cursor style");
    }
}

impl GpuSurfaceWindow for BrowserWindow {
    fn surface(&mut self) -> &mut dyn SurfaceProvider {
        &mut self.surface
    }
}

fn find_or_create_canvas(document: &Document) -> HtmlCanvasElement {
    if let Some(element) = document.get_element_by_id("waterui-canvas") {
        return element
            .dyn_into::<HtmlCanvasElement>()
            .expect("hydrolysis web platform: #waterui-canvas is not a <canvas>");
    }

    let canvas = document
        .create_element("canvas")
        .expect("hydrolysis web platform: failed to create canvas")
        .dyn_into::<HtmlCanvasElement>()
        .expect("hydrolysis web platform: created node is not a canvas");
    canvas.set_id("waterui-canvas");
    let style = canvas.style();
    // The layout viewport exactly: `100vh` is taller than the visible area on
    // mobile browsers, which makes the page itself pannable under the canvas.
    for (property, value) in [
        ("display", "block"),
        ("position", "fixed"),
        ("inset", "0"),
        ("width", "100%"),
        ("height", "100%"),
    ] {
        style
            .set_property(property, value)
            .expect("hydrolysis web platform: failed to style canvas");
    }
    document
        .body()
        .expect("hydrolysis web platform: document body unavailable")
        .append_child(&canvas)
        .expect("hydrolysis web platform: failed to append canvas to body");
    canvas
}

/// Makes the page's canvas the runtime's input surface, whether the page
/// supplied it or the runtime created it.
///
/// - `touch-action: none`: Hydrolysis recognizes every gesture itself, so
///   the browser must never claim a touch for panning or zooming; when it
///   does, it cancels the pointer (`pointercancel` instead of `pointerup`)
///   and a tap never reaches the view under it.
/// - `tabindex` 0: the canvas receives the keys, which a focusable element
///   alone can do.
/// - `outline: none`: Hydrolysis draws focus itself.
fn prepare_canvas(canvas: &HtmlCanvasElement) {
    canvas.set_tab_index(0);
    let style = canvas.style();
    for (property, value) in [("touch-action", "none"), ("outline", "none")] {
        style
            .set_property(property, value)
            .expect("hydrolysis web platform: failed to style the canvas");
    }
}

/// A hidden element whose padding resolves the page's
/// `env(safe-area-inset-*)`. CSS environment variables have no script API,
/// so the probe's computed padding is how the host reads the safe area.
fn create_safe_area_probe(document: &Document) -> HtmlElement {
    let probe = document
        .create_element("div")
        .expect("hydrolysis web platform: failed to create the safe-area probe")
        .dyn_into::<HtmlElement>()
        .expect("hydrolysis web platform: created node is not an HTML element");
    probe
        .set_attribute("aria-hidden", "true")
        .expect("hydrolysis web platform: failed to hide the safe-area probe");
    let style = probe.style();
    for (property, value) in [
        ("position", "fixed"),
        ("inset", "0"),
        ("visibility", "hidden"),
        ("pointer-events", "none"),
        (
            "padding",
            "env(safe-area-inset-top) env(safe-area-inset-right) \
             env(safe-area-inset-bottom) env(safe-area-inset-left)",
        ),
    ] {
        style
            .set_property(property, value)
            .expect("hydrolysis web platform: failed to style the safe-area probe");
    }
    document
        .body()
        .expect("hydrolysis web platform: document body unavailable")
        .append_child(&probe)
        .expect("hydrolysis web platform: failed to append the safe-area probe");
    probe
}

/// The page's safe area in CSS pixels, which are the window's logical units.
fn read_safe_area(
    browser_window: &BrowserHostWindow,
    probe: &HtmlElement,
) -> waterui_layout::padding::EdgeInsets {
    let style = browser_window
        .get_computed_style(probe)
        .expect("hydrolysis web platform: failed to compute the safe-area probe style")
        .expect("hydrolysis web platform: the safe-area probe has no computed style");
    let inset = |property: &str| -> f32 {
        let value = style
            .get_property_value(property)
            .expect("hydrolysis web platform: failed to read a safe-area inset");
        value
            .strip_suffix("px")
            .and_then(|pixels| pixels.parse::<f32>().ok())
            .unwrap_or_else(|| {
                panic!("hydrolysis web platform: safe-area {property} resolved to {value:?}, not a pixel length")
            })
    };
    waterui_layout::padding::EdgeInsets::new(
        inset("padding-top"),
        inset("padding-bottom"),
        inset("padding-left"),
        inset("padding-right"),
    )
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

fn measure_canvas(browser_window: &BrowserHostWindow, canvas: &HtmlCanvasElement) -> PendingResize {
    let scale_factor = browser_window.device_pixel_ratio();
    assert!(
        scale_factor.is_finite() && scale_factor > 0.0,
        "hydrolysis web platform received invalid devicePixelRatio {scale_factor}"
    );

    let rect = canvas.get_bounding_client_rect();
    let mut logical_width = rect.width();
    let mut logical_height = rect.height();
    if logical_width <= 0.0 {
        logical_width = f64::from(canvas.client_width());
    }
    if logical_height <= 0.0 {
        logical_height = f64::from(canvas.client_height());
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

fn apply_canvas_resize(canvas: &HtmlCanvasElement, resize: PendingResize) {
    canvas.set_width(resize.width.max(1));
    canvas.set_height(resize.height.max(1));
}

#[allow(clippy::too_many_arguments)]
#[expect(
    clippy::too_many_lines,
    reason = "the listener registration enumerates every DOM event hook once; the length is the enumeration, not logic"
)]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the shared Rc state is moved into listener closures; borrowing would fight the closure 'static bounds"
)]
fn register_listeners(
    browser_window: &BrowserHostWindow,
    canvas: &HtmlCanvasElement,
    ime_input: &HtmlInputElement,
    pending_events: Rc<RefCell<Vec<InputEvent>>>,
    redraw_requested: Rc<Cell<bool>>,
    scale_factor: Rc<Cell<f64>>,
    pending_resize: Rc<Cell<Option<PendingResize>>>,
    schedule_frame: Rc<dyn Fn()>,
) -> Vec<Closure<dyn FnMut(Event)>> {
    let mut listeners = Vec::new();
    let composing = Rc::new(Cell::new(false));
    let suppress_next_input = Rc::new(Cell::new(false));
    let browser_window_target: EventTarget =
        <BrowserHostWindow as AsRef<EventTarget>>::as_ref(browser_window).clone();
    let canvas_target: EventTarget =
        <HtmlCanvasElement as AsRef<EventTarget>>::as_ref(canvas).clone();
    let ime_input_target: EventTarget =
        <HtmlInputElement as AsRef<EventTarget>>::as_ref(ime_input).clone();

    {
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let schedule_frame = schedule_frame.clone();
        listeners.push(add_event_listener(
            &canvas_target,
            "keydown",
            move |event| {
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
            },
        ));
    }

    {
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let schedule_frame = schedule_frame.clone();
        listeners.push(add_event_listener(&canvas_target, "keyup", move |event| {
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
        let browser_window = browser_window.clone();
        let canvas = canvas.clone();
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        // The original `scale_factor`/`pending_resize` move straight into
        // this closure — nothing after the block needs their Rc handle.
        let schedule_frame = schedule_frame.clone();
        listeners.push(add_event_listener(
            &browser_window_target,
            "resize",
            move |_event| {
                let resize = measure_canvas(&browser_window, &canvas);
                scale_factor.set(resize.scale_factor);
                apply_canvas_resize(&canvas, resize);
                pending_resize.set(Some(resize));
                pending_events.borrow_mut().push(InputEvent::Resize {
                    width: resize.width,
                    height: resize.height,
                });
                redraw_requested.set(true);
                schedule_frame();
            },
        ));
    }

    {
        let canvas = canvas.clone();
        let ime_input = ime_input.clone();
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let schedule_frame = schedule_frame.clone();
        listeners.push(add_event_listener(
            &canvas_target,
            "pointerdown",
            move |event| {
                let event = event
                    .dyn_into::<PointerEvent>()
                    .expect("hydrolysis web platform: pointerdown event had unexpected type");
                event.prevent_default();
                // Preventing the default also prevents the press from focusing
                // the canvas, which receives the keys. Text editing owns the
                // hidden input while it holds focus; anything else returns
                // focus to the canvas.
                let editing = canvas
                    .owner_document()
                    .and_then(|document| document.active_element())
                    .is_some_and(|active| active == *ime_input.as_ref());
                if !editing {
                    let _ = canvas.focus();
                }
                let (x, y) = event_position(
                    &canvas,
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
        let canvas = canvas.clone();
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let schedule_frame = schedule_frame.clone();
        listeners.push(add_event_listener(
            &canvas_target,
            "pointerup",
            move |event| {
                let event = event
                    .dyn_into::<PointerEvent>()
                    .expect("hydrolysis web platform: pointerup event had unexpected type");
                event.prevent_default();
                let (x, y) = event_position(
                    &canvas,
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
        let canvas = canvas.clone();
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let schedule_frame = schedule_frame.clone();
        listeners.push(add_event_listener(
            &canvas_target,
            "pointermove",
            move |event| {
                let event = event
                    .dyn_into::<PointerEvent>()
                    .expect("hydrolysis web platform: pointermove event had unexpected type");
                let (x, y) = event_position(
                    &canvas,
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
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let schedule_frame = schedule_frame.clone();
        listeners.push(add_event_listener(
            &canvas_target,
            "pointercancel",
            move |event| {
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
        let canvas = canvas.clone();
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let schedule_frame = schedule_frame.clone();
        listeners.push(add_event_listener(&canvas_target, "wheel", move |event| {
            let event = event
                .dyn_into::<WheelEvent>()
                .expect("hydrolysis web platform: wheel event had unexpected type");
            event.prevent_default();
            let (x, y) = event_position(
                &canvas,
                f64::from(event.client_x()),
                f64::from(event.client_y()),
            );
            // A DOM wheel delta is the change of the scroll offset; hosts
            // report how the content moves (winit's convention), the
            // opposite sign. The browser has already applied the system's
            // natural-scrolling setting to the delta.
            pending_events.borrow_mut().push(InputEvent::Scroll {
                x,
                y,
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

fn add_event_listener(
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

fn event_position(canvas: &HtmlCanvasElement, client_x: f64, client_y: f64) -> (f32, f32) {
    let rect = canvas.get_bounding_client_rect();
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

pub use BrowserWindow as ExportedBrowserWindow;
