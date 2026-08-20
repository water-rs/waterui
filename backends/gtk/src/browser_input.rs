use std::cell::Cell;
use std::rc::Rc;

use gtk4::gdk::{Key, ModifierType};
use gtk4::glib::Propagation;
use gtk4::prelude::*;

/// One pointer sample as GTK reports it.
///
/// Motion and scroll carry the same shape — a position, a delta, and the input
/// state at that instant — so they share one type rather than repeating six
/// positional arguments at every level of the bridge.
#[derive(Debug, Clone, Copy)]
pub struct PointerSample {
    /// Pointer position in widget-local logical pixels.
    pub x: f64,
    /// Pointer position in widget-local logical pixels.
    pub y: f64,
    /// Movement since the previous sample, or the scroll delta.
    pub delta_x: f64,
    /// Movement since the previous sample, or the scroll delta.
    pub delta_y: f64,
    /// Keyboard and button state when the event was delivered.
    pub modifiers: ModifierType,
    /// GTK event timestamp in milliseconds.
    pub time_ms: u32,
}

pub trait GtkBrowserInput: 'static {
    fn pointer_move(&self, sample: PointerSample);
    fn pointer_button(
        &self,
        pressed: bool,
        button: u32,
        x: f64,
        y: f64,
        modifiers: ModifierType,
        time_ms: u32,
    );
    fn scroll(&self, sample: PointerSample, finished: bool);
    fn focus(&self, focused: bool);
    fn key(&self, pressed: bool, keyval: Key, keycode: u32, modifiers: ModifierType, time_ms: u32);
}

/// Last pointer position, shared by the controllers that need it.
///
/// Scroll events carry no coordinates of their own, so they are reported at
/// wherever motion last saw the pointer.
type PointerPosition = Rc<Cell<(f64, f64)>>;

/// Forwards every GTK input event on `area` into `input`.
pub fn install(area: &gtk4::GLArea, input: Rc<dyn GtkBrowserInput>) {
    let position: PointerPosition = Rc::new(Cell::new((0.0, 0.0)));

    install_motion(area, &input, &position);
    install_click(area, &input);
    install_scroll(area, &input, &position);
    install_focus(area, &input);
    install_key(area, input);
}

fn install_motion(
    area: &gtk4::GLArea,
    input: &Rc<dyn GtkBrowserInput>,
    position: &PointerPosition,
) {
    let previous = Rc::new(Cell::new(None::<(f64, f64)>));

    let motion = gtk4::EventControllerMotion::new();
    motion.connect_motion({
        let input = Rc::clone(input);
        let position = Rc::clone(position);
        let previous = Rc::clone(&previous);
        move |controller, x, y| {
            let (delta_x, delta_y) = previous
                .replace(Some((x, y)))
                .map_or((0.0, 0.0), |(old_x, old_y)| (x - old_x, y - old_y));
            position.set((x, y));
            input.pointer_move(PointerSample {
                x,
                y,
                delta_x,
                delta_y,
                modifiers: controller.current_event_state(),
                time_ms: controller.current_event_time(),
            });
        }
    });
    motion.connect_leave({
        let previous = Rc::clone(&previous);
        move |_| previous.set(None)
    });
    area.add_controller(motion);
}

fn install_click(area: &gtk4::GLArea, input: &Rc<dyn GtkBrowserInput>) {
    let click = gtk4::GestureClick::new();
    click.set_button(0);
    click.connect_pressed({
        let area = area.clone();
        let input = Rc::clone(input);
        move |gesture, _, x, y| {
            area.grab_focus();
            input.focus(true);
            input.pointer_button(
                true,
                gesture.current_button(),
                x,
                y,
                gesture.current_event_state(),
                gesture.current_event_time(),
            );
        }
    });
    click.connect_released({
        let input = Rc::clone(input);
        move |gesture, _, x, y| {
            input.pointer_button(
                false,
                gesture.current_button(),
                x,
                y,
                gesture.current_event_state(),
                gesture.current_event_time(),
            );
        }
    });
    area.add_controller(click);
}

fn install_scroll(
    area: &gtk4::GLArea,
    input: &Rc<dyn GtkBrowserInput>,
    position: &PointerPosition,
) {
    let scroll = gtk4::EventControllerScroll::new(
        gtk4::EventControllerScrollFlags::BOTH_AXES | gtk4::EventControllerScrollFlags::KINETIC,
    );
    scroll.connect_scroll({
        let input = Rc::clone(input);
        let position = Rc::clone(position);
        move |controller, delta_x, delta_y| {
            let (x, y) = position.get();
            input.scroll(
                PointerSample {
                    x,
                    y,
                    delta_x,
                    delta_y,
                    modifiers: controller.current_event_state(),
                    time_ms: controller.current_event_time(),
                },
                false,
            );
            Propagation::Stop
        }
    });
    scroll.connect_scroll_end({
        let input = Rc::clone(input);
        let position = Rc::clone(position);
        move |controller| {
            let (x, y) = position.get();
            input.scroll(
                PointerSample {
                    x,
                    y,
                    delta_x: 0.0,
                    delta_y: 0.0,
                    modifiers: controller.current_event_state(),
                    time_ms: controller.current_event_time(),
                },
                true,
            );
        }
    });
    area.add_controller(scroll);
}

fn install_focus(area: &gtk4::GLArea, input: &Rc<dyn GtkBrowserInput>) {
    let focus = gtk4::EventControllerFocus::new();
    focus.connect_enter({
        let input = Rc::clone(input);
        move |_| input.focus(true)
    });
    focus.connect_leave({
        let input = Rc::clone(input);
        move |_| input.focus(false)
    });
    area.add_controller(focus);
}

fn install_key(area: &gtk4::GLArea, input: Rc<dyn GtkBrowserInput>) {
    let key = gtk4::EventControllerKey::new();
    key.connect_key_pressed({
        let input = Rc::clone(&input);
        move |controller, keyval, keycode, state| {
            input.key(
                true,
                keyval,
                keycode,
                state,
                controller.current_event_time(),
            );
            Propagation::Stop
        }
    });
    key.connect_key_released(move |controller, keyval, keycode, state| {
        input.key(
            false,
            keyval,
            keycode,
            state,
            controller.current_event_time(),
        );
    });
    area.add_controller(key);
}
