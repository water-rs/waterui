//! `GtkGLArea` input, translated into the backend-neutral surface vocabulary.
//!
//! A GPU view that draws its own interactive content — an embedded browser
//! page, a terminal, an editor — reports
//! [`wants_input_events`](waterui_graphics::gpu_surface::GpuView::wants_input_events),
//! and GTK's whole job is then to say what happened in that vocabulary: nothing
//! here knows what a Chromium wheel notch is worth or how `WPEPlatform` packs a
//! modifier word, and adding another such view adds no translation code at all.
//!
//! GTK reaches those views this way rather than through the renderer because
//! its input arrives at the `GtkGLArea`'s own event controllers, not through a
//! renderer that hit-tests surface layers.

use std::cell::Cell;
use std::rc::Rc;
use std::str::FromStr as _;

use gtk4::gdk::{Key as GdkKey, ModifierType};
use gtk4::glib::Propagation;
use gtk4::prelude::*;
use kurbo::Point;
use waterui_graphics::input::{
    Code, Key, Modifiers, NamedKey, ScrollUnit, SurfaceInputEvent, SurfacePointerButton,
};

/// One input-taking surface, as the widget layer sees it.
pub(crate) trait SurfaceInputSink: 'static {
    /// Applies one translated event to the engine.
    fn handle(&self, event: &SurfaceInputEvent);
}

/// Last pointer position, shared by the controllers that need it.
///
/// Scroll events carry no coordinates of their own, so they are reported at
/// wherever motion last saw the pointer.
type PointerPosition = Rc<Cell<Point>>;

/// Forwards every GTK input event on `area` into `input`.
pub(crate) fn install(area: &gtk4::GLArea, input: Rc<dyn SurfaceInputSink>) {
    let position: PointerPosition = Rc::new(Cell::new(Point::ZERO));

    install_motion(area, &input, &position);
    install_click(area, &input);
    install_scroll(area, &input, &position);
    install_focus(area, &input);
    install_key(area, input);
}

/// GTK reports the modifier chord on every event; the surface vocabulary
/// reports it when it changes, so publish it before the event carrying it.
fn send_modifiers(input: &Rc<dyn SurfaceInputSink>, modifiers: ModifierType) {
    input.handle(&SurfaceInputEvent::Modifiers(surface_modifiers(modifiers)));
}

fn install_motion(
    area: &gtk4::GLArea,
    input: &Rc<dyn SurfaceInputSink>,
    position: &PointerPosition,
) {
    let motion = gtk4::EventControllerMotion::new();
    motion.connect_motion({
        let input = Rc::clone(input);
        let position = Rc::clone(position);
        move |controller, x, y| {
            position.set(Point::new(x, y));
            send_modifiers(&input, controller.current_event_state());
            input.handle(&SurfaceInputEvent::PointerMove {
                position: Point::new(x, y),
            });
        }
    });
    area.add_controller(motion);
}

fn install_click(area: &gtk4::GLArea, input: &Rc<dyn SurfaceInputSink>) {
    let click = gtk4::GestureClick::new();
    click.set_button(0);
    click.connect_pressed({
        let area = area.clone();
        let input = Rc::clone(input);
        move |gesture, _, x, y| {
            area.grab_focus();
            input.handle(&SurfaceInputEvent::Focus(true));
            send_modifiers(&input, gesture.current_event_state());
            input.handle(&SurfaceInputEvent::PointerButton {
                pressed: true,
                button: surface_pointer_button(gesture.current_button()),
                position: Point::new(x, y),
            });
        }
    });
    click.connect_released({
        let input = Rc::clone(input);
        move |gesture, _, x, y| {
            send_modifiers(&input, gesture.current_event_state());
            input.handle(&SurfaceInputEvent::PointerButton {
                pressed: false,
                button: surface_pointer_button(gesture.current_button()),
                position: Point::new(x, y),
            });
        }
    });
    area.add_controller(click);
}

fn install_scroll(
    area: &gtk4::GLArea,
    input: &Rc<dyn SurfaceInputSink>,
    position: &PointerPosition,
) {
    let scroll = gtk4::EventControllerScroll::new(
        gtk4::EventControllerScrollFlags::BOTH_AXES | gtk4::EventControllerScrollFlags::KINETIC,
    );
    scroll.connect_scroll({
        let input = Rc::clone(input);
        let position = Rc::clone(position);
        move |controller, delta_x, delta_y| {
            send_modifiers(&input, controller.current_event_state());
            input.handle(&SurfaceInputEvent::Scroll {
                position: position.get(),
                delta_x,
                delta_y,
                // GTK's scroll controller counts wheel notches, not pixels, on
                // both axes; a kinetic glide is delivered in fractions of one.
                unit: ScrollUnit::Line,
                finished: false,
            });
            Propagation::Stop
        }
    });
    scroll.connect_scroll_end({
        let input = Rc::clone(input);
        let position = Rc::clone(position);
        move |controller| {
            send_modifiers(&input, controller.current_event_state());
            input.handle(&SurfaceInputEvent::Scroll {
                position: position.get(),
                delta_x: 0.0,
                delta_y: 0.0,
                unit: ScrollUnit::Line,
                finished: true,
            });
        }
    });
    area.add_controller(scroll);
}

fn install_focus(area: &gtk4::GLArea, input: &Rc<dyn SurfaceInputSink>) {
    let focus = gtk4::EventControllerFocus::new();
    focus.connect_enter({
        let input = Rc::clone(input);
        move |_| input.handle(&SurfaceInputEvent::Focus(true))
    });
    focus.connect_leave({
        let input = Rc::clone(input);
        move |_| input.handle(&SurfaceInputEvent::Focus(false))
    });
    area.add_controller(focus);
}

fn install_key(area: &gtk4::GLArea, input: Rc<dyn SurfaceInputSink>) {
    let key = gtk4::EventControllerKey::new();
    key.connect_key_pressed({
        let input = Rc::clone(&input);
        move |_, keyval, keycode, state| {
            input.handle(&surface_key_event(true, keyval, keycode, state));
            Propagation::Stop
        }
    });
    key.connect_key_released(move |_, keyval, keycode, state| {
        input.handle(&surface_key_event(false, keyval, keycode, state));
    });
    area.add_controller(key);
}

/// A key event carries its own modifier chord, so it needs no separate
/// [`SurfaceInputEvent::Modifiers`] before it.
fn surface_key_event(
    pressed: bool,
    keyval: GdkKey,
    keycode: u32,
    modifiers: ModifierType,
) -> SurfaceInputEvent {
    SurfaceInputEvent::Key {
        pressed,
        key: surface_key(keyval),
        code: surface_code(keycode),
        modifiers: surface_modifiers(modifiers),
        // GDK's key controller does not distinguish an auto-repeat press from a
        // fresh one.
        repeat: false,
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
    result.set(
        Modifiers::CAPS_LOCK,
        modifiers.contains(ModifierType::LOCK_MASK),
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
