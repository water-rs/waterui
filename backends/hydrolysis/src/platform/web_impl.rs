use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use nami::Signal;
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{
    CompositionEvent, Document, Event, EventTarget, HtmlCanvasElement, HtmlInputElement, KeyboardEvent,
    PointerEvent, WheelEvent, Window as BrowserHostWindow,
};

use super::{
    CursorStyle, InputEvent, KeyCode, KeyState, Modifiers, PlatformWindow, PointerButton,
    SurfaceError, SurfaceFrame, SurfaceProvider, TextInputPurpose, TextInputState, WindowState,
    WuiWindow, select_hydrolysis_surface_format,
};

#[derive(Clone, Copy)]
struct PendingResize {
    width: u32,
    height: u32,
    scale_factor: f64,
}

pub struct BrowserSurface {
    _instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
}

impl core::fmt::Debug for BrowserSurface {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BrowserSurface")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl BrowserSurface {
    pub async fn new(canvas: HtmlCanvasElement, width: u32, height: u32) -> Self {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
            .expect("hydrolysis web surface: failed to create canvas surface");
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
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

        let caps = surface.get_capabilities(&adapter);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: select_hydrolysis_surface_format(&caps),
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        Self {
            _instance: instance,
            surface,
            device,
            queue,
            config,
        }
    }
}

impl SurfaceProvider for BrowserSurface {
    fn device(&self) -> &wgpu::Device {
        &self.device
    }

    fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    fn acquire(&mut self) -> Result<SurfaceFrame, SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        Ok(SurfaceFrame::Browser { output, view })
    }

    fn present(&mut self, frame: SurfaceFrame) {
        match frame {
            SurfaceFrame::Browser { output, .. } => output.present(),
            SurfaceFrame::Offscreen { .. } => {
                panic!("hydrolysis web surface received an offscreen frame")
            }
            #[cfg(feature = "winit")]
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
}

pub struct BrowserWindow {
    browser_window: BrowserHostWindow,
    document: Document,
    canvas: HtmlCanvasElement,
    ime_input: HtmlInputElement,
    surface: BrowserSurface,
    pending_events: Rc<RefCell<Vec<InputEvent>>>,
    redraw_requested: Rc<Cell<bool>>,
    scale_factor: Rc<Cell<f64>>,
    pending_resize: Rc<Cell<Option<PendingResize>>>,
    current_cursor_style: CursorStyle,
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
    pub async fn new(schedule_frame: Rc<dyn Fn()>) -> Self {
        let browser_window = web_sys::window()
            .expect("hydrolysis web platform: browser window unavailable");
        let document = browser_window
            .document()
            .expect("hydrolysis web platform: document unavailable");
        let canvas = find_or_create_canvas(&document);
        let ime_input = find_or_create_ime_input(&document);
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

        let surface = BrowserSurface::new(
            canvas.clone(),
            initial_resize.width,
            initial_resize.height,
        )
        .await;

        let listeners = register_listeners(
            &browser_window,
            &canvas,
            &ime_input,
            pending_events.clone(),
            redraw_requested.clone(),
            scale_factor.clone(),
            pending_resize.clone(),
            schedule_frame,
        );

        Self {
            browser_window,
            document,
            canvas,
            ime_input,
            surface,
            pending_events,
            redraw_requested,
            scale_factor,
            pending_resize,
            current_cursor_style: CursorStyle::Arrow,
            _listeners: listeners,
        }
    }

    pub fn take_redraw_request(&self) -> bool {
        self.redraw_requested.replace(false)
    }
}

impl PlatformWindow for BrowserWindow {
    fn surface(&mut self) -> &mut dyn SurfaceProvider {
        &mut self.surface
    }

    fn apply_properties(&mut self, window: &WuiWindow) {
        self.document.set_title(window.title.get().as_str());

        match window.state.get() {
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
        }

        if self.canvas.client_width() == 0 || self.canvas.client_height() == 0 {
            let frame = window.frame.get();
            self.canvas
                .style()
                .set_property("width", &format!("{}px", frame.width().max(1.0)))
                .expect("hydrolysis web platform: failed to set fallback canvas width");
            self.canvas
                .style()
                .set_property("height", &format!("{}px", frame.height().max(1.0)))
                .expect("hydrolysis web platform: failed to set fallback canvas height");

            let resize = measure_canvas(&self.browser_window, &self.canvas);
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

    fn request_redraw(&self) {
        self.redraw_requested.set(true);
    }

    fn scale_factor(&self) -> f64 {
        self.scale_factor.get()
    }

    fn sync_text_input_state(&mut self, state: Option<TextInputState>) {
        match state {
            Some(state) => {
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
            }
            None => {
                self.ime_input.set_value("");
                let _ = self.ime_input.blur();
            }
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
    style
        .set_property("display", "block")
        .expect("hydrolysis web platform: failed to style canvas");
    style
        .set_property("width", "100vw")
        .expect("hydrolysis web platform: failed to style canvas width");
    style
        .set_property("height", "100vh")
        .expect("hydrolysis web platform: failed to style canvas height");
    document
        .body()
        .expect("hydrolysis web platform: document body unavailable")
        .append_child(&canvas)
        .expect("hydrolysis web platform: failed to append canvas to body");
    canvas
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
        !(!scale_factor.is_finite() || scale_factor <= 0.0),
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
        width: (logical_width.max(1.0) * scale_factor).round() as u32,
        height: (logical_height.max(1.0) * scale_factor).round() as u32,
        scale_factor,
    }
}

fn apply_canvas_resize(canvas: &HtmlCanvasElement, resize: PendingResize) {
    canvas.set_width(resize.width.max(1));
    canvas.set_height(resize.height.max(1));
}

#[allow(clippy::too_many_arguments)]
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
        let browser_window = browser_window.clone();
        let canvas = canvas.clone();
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let scale_factor = scale_factor.clone();
        let pending_resize = pending_resize.clone();
        let schedule_frame = schedule_frame.clone();
        listeners.push(add_event_listener(&browser_window_target, "resize", move |_event| {
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
        }));
    }

    {
        let canvas = canvas.clone();
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let schedule_frame = schedule_frame.clone();
        listeners.push(add_event_listener(&canvas_target, "pointerdown", move |event| {
            let event = event
                .dyn_into::<PointerEvent>()
                .expect("hydrolysis web platform: pointerdown event had unexpected type");
            event.prevent_default();
            let (x, y) = event_position(
                &canvas,
                f64::from(event.client_x()),
                f64::from(event.client_y()),
            );
            pending_events.borrow_mut().push(InputEvent::PointerDown {
                x,
                y,
                button: map_pointer_button(event.button()),
            });
            redraw_requested.set(true);
            schedule_frame();
        }));
    }

    {
        let canvas = canvas.clone();
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let schedule_frame = schedule_frame.clone();
        listeners.push(add_event_listener(&canvas_target, "pointerup", move |event| {
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
                x,
                y,
                button: map_pointer_button(event.button()),
            });
            redraw_requested.set(true);
            schedule_frame();
        }));
    }

    {
        let canvas = canvas.clone();
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let schedule_frame = schedule_frame.clone();
        listeners.push(add_event_listener(&canvas_target, "pointermove", move |event| {
            let event = event
                .dyn_into::<PointerEvent>()
                .expect("hydrolysis web platform: pointermove event had unexpected type");
            let (x, y) = event_position(
                &canvas,
                f64::from(event.client_x()),
                f64::from(event.client_y()),
            );
            pending_events
                .borrow_mut()
                .push(InputEvent::PointerMove { x, y });
            redraw_requested.set(true);
            schedule_frame();
        }));
    }

    {
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let schedule_frame = schedule_frame.clone();
        listeners.push(add_event_listener(&canvas_target, "pointercancel", move |_event| {
            pending_events.borrow_mut().push(InputEvent::PointerCancel);
            redraw_requested.set(true);
            schedule_frame();
        }));
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
            pending_events.borrow_mut().push(InputEvent::Scroll {
                x,
                y,
                dx: event.delta_x() as f32,
                dy: event.delta_y() as f32,
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
        listeners.push(add_event_listener(ime_input.as_ref(), "keydown", move |event| {
            let event = event
                .dyn_into::<KeyboardEvent>()
                .expect("hydrolysis web platform: keydown event had unexpected type");
            let modifiers = map_modifiers_from_keyboard(&event);
            match event.key().as_str() {
                "Backspace" => {
                    event.prevent_default();
                    pending_events.borrow_mut().push(InputEvent::Key {
                        key: KeyCode::Named("Backspace".to_string()),
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
                    pending_events.borrow_mut().push(InputEvent::ImeCommit {
                        text: "\t".to_string(),
                    });
                }
                _ => return,
            }
            redraw_requested.set(true);
            schedule_frame();
        }));
    }

    {
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let schedule_frame = schedule_frame.clone();
        let ime_input = ime_input.clone();
        let composing = composing.clone();
        let suppress_next_input = suppress_next_input.clone();
        listeners.push(add_event_listener(&ime_input_target, "input", move |event| {
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
        }));
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
        let composing = composing.clone();
        let suppress_next_input = suppress_next_input.clone();
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
        let pending_events = pending_events.clone();
        let redraw_requested = redraw_requested.clone();
        let schedule_frame = schedule_frame.clone();
        listeners.push(add_event_listener(&ime_input_target, "blur", move |_event| {
            pending_events.borrow_mut().push(InputEvent::ImeDisabled);
            redraw_requested.set(true);
            schedule_frame();
        }));
    }

    listeners
}

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
        .unwrap_or_else(|_| panic!("hydrolysis web platform: failed to register {event_name} listener"));
    closure
}

fn event_position(canvas: &HtmlCanvasElement, client_x: f64, client_y: f64) -> (f32, f32) {
    let rect = canvas.get_bounding_client_rect();
    ((client_x - rect.left()) as f32, (client_y - rect.top()) as f32)
}

fn map_modifiers_from_keyboard(event: &KeyboardEvent) -> Modifiers {
    Modifiers {
        shift: event.shift_key(),
        control: event.ctrl_key(),
        alt: event.alt_key(),
        super_key: event.meta_key(),
    }
}

fn map_pointer_button(button: i16) -> PointerButton {
    match button {
        0 => PointerButton::Primary,
        1 => PointerButton::Middle,
        2 => PointerButton::Secondary,
        3 => PointerButton::Back,
        4 => PointerButton::Forward,
        other if other >= 0 => PointerButton::Other(other as u16),
        _ => PointerButton::Other(u16::MAX),
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
