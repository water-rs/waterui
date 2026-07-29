use std::cell::Cell;
use std::rc::Rc;

use gtk4::gdk::{Key, ModifierType};
use gtk4::glib::Propagation;
use gtk4::prelude::*;

pub(crate) trait GtkBrowserInput: 'static {
    fn pointer_move(
        &self,
        x: f64,
        y: f64,
        delta_x: f64,
        delta_y: f64,
        modifiers: ModifierType,
        time_ms: u32,
    );
    fn pointer_button(
        &self,
        pressed: bool,
        button: u32,
        x: f64,
        y: f64,
        modifiers: ModifierType,
        time_ms: u32,
    );
    fn scroll(
        &self,
        x: f64,
        y: f64,
        delta_x: f64,
        delta_y: f64,
        finished: bool,
        modifiers: ModifierType,
        time_ms: u32,
    );
    fn focus(&self, focused: bool);
    fn key(&self, pressed: bool, keyval: Key, keycode: u32, modifiers: ModifierType, time_ms: u32);
}

pub(crate) fn install(area: &gtk4::GLArea, input: Rc<dyn GtkBrowserInput>) {
    let position = Rc::new(Cell::new((0.0, 0.0)));
    let previous = Rc::new(Cell::new(None::<(f64, f64)>));

    let motion = gtk4::EventControllerMotion::new();
    motion.connect_motion({
        let input = Rc::clone(&input);
        let position = Rc::clone(&position);
        let previous = Rc::clone(&previous);
        move |controller, x, y| {
            let (delta_x, delta_y) = previous
                .replace(Some((x, y)))
                .map_or((0.0, 0.0), |(old_x, old_y)| (x - old_x, y - old_y));
            position.set((x, y));
            input.pointer_move(
                x,
                y,
                delta_x,
                delta_y,
                controller.current_event_state(),
                controller.current_event_time(),
            );
        }
    });
    motion.connect_leave({
        let previous = Rc::clone(&previous);
        move |_| previous.set(None)
    });
    area.add_controller(motion);

    let click = gtk4::GestureClick::new();
    click.set_button(0);
    click.connect_pressed({
        let area = area.clone();
        let input = Rc::clone(&input);
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
        let input = Rc::clone(&input);
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

    let scroll = gtk4::EventControllerScroll::new(
        gtk4::EventControllerScrollFlags::BOTH_AXES | gtk4::EventControllerScrollFlags::KINETIC,
    );
    scroll.connect_scroll({
        let input = Rc::clone(&input);
        let position = Rc::clone(&position);
        move |controller, delta_x, delta_y| {
            let (x, y) = position.get();
            input.scroll(
                x,
                y,
                delta_x,
                delta_y,
                false,
                controller.current_event_state(),
                controller.current_event_time(),
            );
            Propagation::Stop
        }
    });
    scroll.connect_scroll_end({
        let input = Rc::clone(&input);
        let position = Rc::clone(&position);
        move |controller| {
            let (x, y) = position.get();
            input.scroll(
                x,
                y,
                0.0,
                0.0,
                true,
                controller.current_event_state(),
                controller.current_event_time(),
            );
        }
    });
    area.add_controller(scroll);

    let focus = gtk4::EventControllerFocus::new();
    focus.connect_enter({
        let input = Rc::clone(&input);
        move |_| input.focus(true)
    });
    focus.connect_leave({
        let input = Rc::clone(&input);
        move |_| input.focus(false)
    });
    area.add_controller(focus);

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
