use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use gtk4::prelude::*;
use waterui_browser_cef::{
    CefPageHandle, CefRuntime, CefRuntimeConfiguration, CefSurfaceInput, CefViewport, gpu_view,
};
use waterui_core::Environment;
use waterui_graphics::gpu_surface::GpuSurface;
use waterui_graphics::input::SurfaceInputEvent;

use crate::browser_input::{SurfaceInputSink, install};

pub(crate) fn ensure_runtime(env: &mut Environment) -> CefRuntime {
    let runtime = env
        .get::<CefRuntime>()
        .cloned()
        .unwrap_or_else(|| CefRuntime::initialize(CefRuntimeConfiguration::packaged()));
    env.insert(runtime.clone());
    #[cfg(feature = "webview-cef")]
    env.insert(runtime.webview_controller());
    #[cfg(feature = "chromium")]
    env.insert(runtime.chromium_controller());
    runtime
}

pub(crate) fn start_message_pump(runtime: CefRuntime) {
    executor_core::spawn_local(async move {
        loop {
            let deadline = runtime.pump().instant();
            glib::timeout_future(deadline.saturating_duration_since(Instant::now())).await;
        }
    })
    .detach();
}

pub(crate) fn render_page(
    page: CefPageHandle,
    env: &Environment,
    input_enabled: bool,
) -> gtk4::Widget {
    let viewport = CefViewport::new();
    let surface = GpuSurface::new(gpu_view(page.clone(), viewport.clone()));
    let widget = crate::components::graphics::gpu_surface::render_gpu_surface(surface, env.clone());
    let area = widget
        .clone()
        .downcast::<gtk4::GLArea>()
        .expect("CEF GpuSurface must render as GtkGLArea");
    area.set_focusable(input_enabled);
    viewport.set_scale(f64::from(area.scale_factor().max(1)));
    area.connect_scale_factor_notify(move |area| {
        viewport.set_scale(f64::from(area.scale_factor().max(1)));
        area.queue_render();
    });
    if input_enabled {
        install(&area, Rc::new(CefGtkInput::new(page)));
    }
    widget
}

/// Hands the `GtkGLArea`'s translated input to the CEF engine crate.
///
/// Nothing Chromium-specific lives in this backend: the wheel unit, the
/// virtual-key table, the modifier word, the pressed-button state and the
/// editing shortcuts are all [`CefSurfaceInput`]'s, shared with every other
/// backend that embeds a CEF page.
struct CefGtkInput {
    input: RefCell<CefSurfaceInput>,
}

impl CefGtkInput {
    fn new(page: CefPageHandle) -> Self {
        Self {
            input: RefCell::new(CefSurfaceInput::new(page)),
        }
    }
}

impl SurfaceInputSink for CefGtkInput {
    fn handle(&self, event: &SurfaceInputEvent) {
        self.input.borrow_mut().handle(event);
    }
}
