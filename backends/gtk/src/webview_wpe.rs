//! Bundled WPE `WebKit` implementation for the GTK backend.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use waterui_browser_wpe::{WpeController, WpeGpuView, WpePage, WpeSurfaceInput, WpeViewport};
use waterui_core::Environment;
use waterui_graphics::gpu_surface::GpuSurface;
use waterui_graphics::input::SurfaceInputEvent;
use waterui_webview::WebViewController;

use crate::browser_input::{SurfaceInputSink, install};

pub use waterui_browser_wpe::WpeWebViewHandle as GtkWebViewHandle;

/// Installs the bundled WPE controller before the application view is built,
/// unless the application already provided one.
///
/// The backend's controller is a default, not an override — the name said
/// "ensure" while the body overwrote whatever was already there.
pub fn ensure_webview_controller(env: &mut Environment) {
    if env.get::<WebViewController>().is_some() {
        return;
    }
    env.insert(WebViewController::new(WpeController::packaged()));
}

/// Creates a GPU-only WPE widget and forwards GTK input into `WPEPlatform`.
pub(crate) fn render_webview(handle: &GtkWebViewHandle, env: &Environment) -> gtk4::Widget {
    let page = handle.page().clone();
    let viewport = WpeViewport::new();
    let surface = GpuSurface::new(WpeGpuView::with_viewport(page.clone(), viewport.clone()));
    let widget = crate::components::graphics::gpu_surface::render_gpu_surface(surface, env.clone());
    let area = widget
        .clone()
        .downcast::<gtk4::GLArea>()
        .expect("WPE GpuSurface must render as GtkGLArea");
    area.set_focusable(true);
    viewport.set_scale(f64::from(area.scale_factor().max(1)));
    area.connect_scale_factor_notify(move |area| {
        viewport.set_scale(f64::from(area.scale_factor().max(1)));
        area.queue_render();
    });
    install(&area, Rc::new(WpeGtkInput::new(page)));
    widget
}

/// Hands the `GtkGLArea`'s translated input to the WPE engine crate.
///
/// Nothing `WPEPlatform`-specific lives in this backend: the modifier word, the
/// button numbering, the XKB keycode and keysym tables and the event clock are
/// all [`WpeSurfaceInput`]'s, shared with every other backend that embeds a WPE
/// page.
struct WpeGtkInput {
    input: RefCell<WpeSurfaceInput>,
}

impl WpeGtkInput {
    fn new(page: WpePage) -> Self {
        Self {
            input: RefCell::new(WpeSurfaceInput::new(page)),
        }
    }
}

impl SurfaceInputSink for WpeGtkInput {
    fn handle(&self, event: &SurfaceInputEvent) {
        self.input.borrow_mut().handle(event);
    }
}
