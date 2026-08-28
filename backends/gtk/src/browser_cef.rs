use std::cell::RefCell;
use std::rc::Rc;
use std::str::FromStr as _;
use std::time::Instant;

use gtk4::gdk::{Key as GdkKey, ModifierType};
use gtk4::prelude::*;
use kurbo::Point;
use waterui_browser_cef::{
    CefPageHandle, CefRuntime, CefRuntimeConfiguration, CefSurfaceInput, CefViewport, gpu_view,
};
use waterui_core::Environment;
use waterui_graphics::gpu_surface::GpuSurface;
use waterui_graphics::input::{
    Code, Key, Modifiers, NamedKey, ScrollUnit, SurfaceInputEvent, SurfacePointerButton,
};

use crate::browser_input::{GtkBrowserInput, PointerSample, install};

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

/// Translates the `GtkGLArea`'s event controllers into the backend-neutral
/// surface vocabulary and hands the result to the CEF engine crate.
///
/// Nothing Chromium-specific lives here: the wheel unit, the virtual-key table,
/// the modifier word, the pressed-button state and the editing shortcuts are
/// all [`CefSurfaceInput`]'s, shared with every other backend that embeds a CEF
/// page. GTK's only job is to say what happened in the vocabulary every
/// interactive surface speaks.
struct CefGtkInput {
    input: RefCell<CefSurfaceInput>,
}

impl CefGtkInput {
    fn new(page: CefPageHandle) -> Self {
        Self {
            input: RefCell::new(CefSurfaceInput::new(page)),
        }
    }

    fn send(&self, event: &SurfaceInputEvent) {
        self.input.borrow_mut().handle(event);
    }

    /// GTK reports the modifier chord on every event; the surface vocabulary
    /// reports it when it changes, so publish it before the event carrying it.
    fn send_modifiers(&self, modifiers: ModifierType) {
        self.send(&SurfaceInputEvent::Modifiers(surface_modifiers(modifiers)));
    }
}

impl GtkBrowserInput for CefGtkInput {
    fn pointer_move(&self, sample: PointerSample) {
        self.send_modifiers(sample.modifiers);
        self.send(&SurfaceInputEvent::PointerMove {
            position: Point::new(sample.x, sample.y),
        });
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
        self.send_modifiers(modifiers);
        self.send(&SurfaceInputEvent::PointerButton {
            pressed,
            button: surface_pointer_button(button),
            position: Point::new(x, y),
        });
    }

    fn scroll(&self, sample: PointerSample, finished: bool) {
        self.send_modifiers(sample.modifiers);
        self.send(&SurfaceInputEvent::Scroll {
            position: Point::new(sample.x, sample.y),
            delta_x: sample.delta_x,
            delta_y: sample.delta_y,
            // GTK's scroll controller counts wheel notches, not pixels, on both
            // axes; a kinetic glide is delivered in fractions of one.
            unit: ScrollUnit::Line,
            finished,
        });
    }

    fn focus(&self, focused: bool) {
        self.send(&SurfaceInputEvent::Focus(focused));
    }

    fn key(
        &self,
        pressed: bool,
        keyval: GdkKey,
        keycode: u32,
        modifiers: ModifierType,
        _time_ms: u32,
    ) {
        self.send(&SurfaceInputEvent::Key {
            pressed,
            key: surface_key(keyval),
            code: surface_code(keycode),
            modifiers: surface_modifiers(modifiers),
            // GDK's key controller does not distinguish an auto-repeat press
            // from a fresh one.
            repeat: false,
        });
    }
}

fn surface_modifiers(modifiers: ModifierType) -> Modifiers {
    let mut result = Modifiers::empty();
    result.set(
        Modifiers::SHIFT,
        modifiers.contains(ModifierType::SHIFT_MASK),
    );
    result.set(
        Modifiers::CONTROL,
        modifiers.contains(ModifierType::CONTROL_MASK),
    );
    result.set(Modifiers::ALT, modifiers.contains(ModifierType::ALT_MASK));
    result.set(
        Modifiers::META,
        modifiers.intersects(
            ModifierType::META_MASK | ModifierType::SUPER_MASK | ModifierType::HYPER_MASK,
        ),
    );
    result
}

/// # Panics
///
/// Panics on a GTK button number outside the five the W3C vocabulary names.
fn surface_pointer_button(button: u32) -> SurfacePointerButton {
    match button {
        1 => SurfacePointerButton::Primary,
        2 => SurfacePointerButton::Middle,
        3 => SurfacePointerButton::Secondary,
        4 => SurfacePointerButton::Back,
        5 => SurfacePointerButton::Forward,
        other => panic!("GTK reported unsupported pointer button {other}"),
    }
}

/// The physical key a GDK hardware keycode denotes.
///
/// GTK reports the XKB keycode, and Chromium's own keycode table names the same
/// physical key in the W3C vocabulary the surface events carry.
fn surface_code(keycode: u32) -> Code {
    let Ok(keycode) = u16::try_from(keycode) else {
        return Code::Unidentified;
    };
    let Ok(map) = keycode::KeyMap::try_from(keycode::KeyMapping::Xkb(keycode)) else {
        return Code::Unidentified;
    };
    map.code
        .and_then(|code| Code::from_str(&code.to_string()).ok())
        .unwrap_or(Code::Unidentified)
}

/// The logical key a GDK keyval denotes.
///
/// A keyval that types something is that character; the rest are named, and GDK
/// names them after their X11 keysyms.
fn surface_key(keyval: GdkKey) -> Key {
    if let Some(character) = keyval.to_unicode()
        && !character.is_control()
    {
        return Key::Character(character.to_string());
    }
    keyval
        .name()
        .and_then(|name| named_key(&name))
        .map_or(Key::Named(NamedKey::Unidentified), Key::Named)
}

/// The W3C name for an X11 keysym name.
///
/// Most keys are spelled the same in both vocabularies, so only the ones that
/// differ are listed; everything else — the function keys, `Home`, `End`,
/// `Insert`, `Delete`, `Escape` — is handed to `NamedKey`'s own parser as is.
fn named_key(name: &str) -> Option<NamedKey> {
    let w3c = match name {
        "BackSpace" => "Backspace",
        "Return" | "KP_Enter" | "ISO_Enter" => "Enter",
        "ISO_Left_Tab" => "Tab",
        "Left" | "KP_Left" => "ArrowLeft",
        "Right" | "KP_Right" => "ArrowRight",
        "Up" | "KP_Up" => "ArrowUp",
        "Down" | "KP_Down" => "ArrowDown",
        "Prior" | "Page_Up" | "KP_Prior" | "KP_Page_Up" => "PageUp",
        "Next" | "Page_Down" | "KP_Next" | "KP_Page_Down" => "PageDown",
        "KP_Home" => "Home",
        "KP_End" => "End",
        "KP_Insert" => "Insert",
        "KP_Delete" => "Delete",
        "Shift_L" | "Shift_R" => "Shift",
        "Control_L" | "Control_R" => "Control",
        "Alt_L" | "Alt_R" => "Alt",
        "Meta_L" | "Meta_R" | "Super_L" | "Super_R" | "Hyper_L" | "Hyper_R" => "Meta",
        "ISO_Level3_Shift" | "ISO_Level5_Shift" => "AltGraph",
        "Caps_Lock" => "CapsLock",
        "Num_Lock" => "NumLock",
        "Scroll_Lock" => "ScrollLock",
        "Print" => "PrintScreen",
        "Menu" => "ContextMenu",
        other => other,
    };
    NamedKey::from_str(w3c).ok()
}
