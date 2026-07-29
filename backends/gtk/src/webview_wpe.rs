//! Bundled WPE WebKit implementation for the GTK backend.

use std::rc::Rc;

use gtk4::gdk::{Key, ModifierType};
use gtk4::glib::translate::IntoGlib as _;
use gtk4::prelude::*;
use waterui_browser_wpe::{PointerButton, WpeController, WpeGpuView, WpePage, WpeViewport};
use waterui_core::Environment;
use waterui_graphics::gpu_surface::GpuSurface;
use waterui_webview::WebViewController;

use crate::browser_input::{GtkBrowserInput, install};

pub use waterui_browser_wpe::WpeWebViewHandle as GtkWebViewHandle;

/// Installs the bundled WPE controller before the application view is built.
pub fn ensure_webview_controller(env: &mut Environment) {
    env.insert(WebViewController::new(WpeController::packaged()));
}

/// Creates a GPU-only WPE widget and forwards GTK input into WPEPlatform.
pub(crate) fn render_webview(handle: &GtkWebViewHandle, env: &Environment) -> gtk4::Widget {
    let page = handle.page().clone();
    let viewport = WpeViewport::new();
    let surface = GpuSurface::new(WpeGpuView::with_external_input(
        page.clone(),
        viewport.clone(),
    ));
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
    install(&area, Rc::new(WpeGtkInput(page)));
    widget
}

struct WpeGtkInput(WpePage);

impl GtkBrowserInput for WpeGtkInput {
    fn pointer_move(
        &self,
        x: f64,
        y: f64,
        delta_x: f64,
        delta_y: f64,
        modifiers: ModifierType,
        time_ms: u32,
    ) {
        self.0
            .pointer_move(x, y, delta_x, delta_y, wpe_modifiers(modifiers), time_ms);
    }

    fn pointer_button(
        &self,
        pressed: bool,
        button: u32,
        x: f64,
        y: f64,
        modifiers: ModifierType,
        time_ms: u32,
    ) {
        self.0.pointer_button(
            pressed,
            pointer_button(button),
            x,
            y,
            wpe_modifiers(modifiers),
            time_ms,
        );
    }

    fn scroll(
        &self,
        x: f64,
        y: f64,
        delta_x: f64,
        delta_y: f64,
        finished: bool,
        modifiers: ModifierType,
        time_ms: u32,
    ) {
        self.0.scroll(
            x,
            y,
            delta_x,
            delta_y,
            true,
            finished,
            wpe_modifiers(modifiers),
            time_ms,
        );
    }

    fn focus(&self, focused: bool) {
        self.0.set_focus(focused);
    }

    fn key(&self, pressed: bool, keyval: Key, keycode: u32, modifiers: ModifierType, time_ms: u32) {
        self.0.key(
            pressed,
            keycode,
            keyval.into_glib(),
            wpe_modifiers(modifiers),
            time_ms,
        );
    }
}

fn pointer_button(button: u32) -> PointerButton {
    match button {
        1 => PointerButton::Primary,
        2 => PointerButton::Middle,
        3 => PointerButton::Secondary,
        4 => PointerButton::Back,
        5 => PointerButton::Forward,
        other => panic!("WPE received unsupported GTK pointer button {other}"),
    }
}

fn wpe_modifiers(modifiers: ModifierType) -> u32 {
    let mut result = 0;
    result |= u32::from(modifiers.contains(ModifierType::CONTROL_MASK));
    result |= u32::from(modifiers.contains(ModifierType::SHIFT_MASK)) << 1;
    result |= u32::from(modifiers.contains(ModifierType::ALT_MASK)) << 2;
    result |=
        u32::from(modifiers.intersects(
            ModifierType::META_MASK | ModifierType::SUPER_MASK | ModifierType::HYPER_MASK,
        )) << 3;
    result |= u32::from(modifiers.contains(ModifierType::LOCK_MASK)) << 4;
    result |= u32::from(modifiers.contains(ModifierType::BUTTON1_MASK)) << 8;
    result |= u32::from(modifiers.contains(ModifierType::BUTTON2_MASK)) << 9;
    result |= u32::from(modifiers.contains(ModifierType::BUTTON3_MASK)) << 10;
    result |= u32::from(modifiers.contains(ModifierType::BUTTON4_MASK)) << 11;
    result |= u32::from(modifiers.contains(ModifierType::BUTTON5_MASK)) << 12;
    result
}
