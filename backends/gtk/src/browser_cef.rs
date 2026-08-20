use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use gtk4::gdk::{Key, ModifierType};
use gtk4::glib::translate::IntoGlib as _;
use gtk4::prelude::*;
use waterui_browser_cef::{
    CefInputModifiers, CefKeyInput, CefPageHandle, CefPointerButton, CefRuntime,
    CefRuntimeConfiguration, CefViewport, gpu_view,
};
use waterui_core::Environment;
use waterui_graphics::gpu_surface::GpuSurface;

use crate::browser_input::{GtkBrowserInput, PointerSample, install};

const CEF_WHEEL_DELTA: f64 = 120.0;

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

struct CefGtkInput {
    page: CefPageHandle,
    buttons: Cell<(bool, bool, bool)>,
    wheel_remainder: Cell<(f64, f64)>,
}

impl CefGtkInput {
    fn new(page: CefPageHandle) -> Self {
        Self {
            page,
            buttons: Cell::new((false, false, false)),
            wheel_remainder: Cell::new((0.0, 0.0)),
        }
    }

    fn modifiers(&self, modifiers: ModifierType) -> CefInputModifiers {
        let (primary_button, middle_button, secondary_button) = self.buttons.get();
        CefInputModifiers {
            shift: modifiers.contains(ModifierType::SHIFT_MASK),
            control: modifiers.contains(ModifierType::CONTROL_MASK),
            alt: modifiers.contains(ModifierType::ALT_MASK),
            command: modifiers.intersects(
                ModifierType::META_MASK | ModifierType::SUPER_MASK | ModifierType::HYPER_MASK,
            ),
            primary_button,
            middle_button,
            secondary_button,
        }
    }

    fn button(button: u32) -> Option<CefPointerButton> {
        match button {
            1 => Some(CefPointerButton::Primary),
            2 => Some(CefPointerButton::Middle),
            3 => Some(CefPointerButton::Secondary),
            4 | 5 => None,
            other => panic!("CEF received unsupported GTK pointer button {other}"),
        }
    }

    fn update_button(&self, button: CefPointerButton, pressed: bool) {
        let (mut primary, mut middle, mut secondary) = self.buttons.get();
        match button {
            CefPointerButton::Primary => primary = pressed,
            CefPointerButton::Middle => middle = pressed,
            CefPointerButton::Secondary => secondary = pressed,
        }
        self.buttons.set((primary, middle, secondary));
    }
}

impl GtkBrowserInput for CefGtkInput {
    fn pointer_move(&self, sample: PointerSample) {
        self.page
            .pointer_move(sample.x, sample.y, self.modifiers(sample.modifiers));
    }

    fn pointer_button(
        &self,
        pressed: bool,
        button: u32,
        x: f64,
        y: f64,
        modifiers: ModifierType,
        _time_ms: u32,
    ) {
        let Some(button) = Self::button(button) else {
            if pressed {
                match button {
                    4 => self.page.go_back(),
                    5 => self.page.go_forward(),
                    _ => unreachable!("only GTK navigation buttons omit a CEF pointer button"),
                }
            }
            return;
        };
        self.update_button(button, pressed);
        self.page
            .pointer_button(pressed, button, x, y, self.modifiers(modifiers));
    }

    fn scroll(&self, sample: PointerSample, finished: bool) {
        if finished {
            return;
        }
        let remainder = self.wheel_remainder.get();
        let delta_x = sample.delta_x.mul_add(CEF_WHEEL_DELTA, remainder.0);
        let delta_y = sample.delta_y.mul_add(CEF_WHEEL_DELTA, remainder.1);
        let integral_x = delta_x.round();
        let integral_y = delta_y.round();
        self.wheel_remainder
            .set((delta_x - integral_x, delta_y - integral_y));
        self.page.scroll(
            sample.x,
            sample.y,
            integral_x,
            integral_y,
            self.modifiers(sample.modifiers),
        );
    }

    fn focus(&self, focused: bool) {
        self.page.set_focus(focused);
    }

    fn key(
        &self,
        pressed: bool,
        keyval: Key,
        keycode: u32,
        modifiers: ModifierType,
        _time_ms: u32,
    ) {
        self.page.key(
            pressed,
            CefKeyInput {
                native_keycode: keycode,
                keyval: keyval.into_glib(),
                character: keyval.to_unicode(),
            },
            self.modifiers(modifiers),
        );
    }
}
