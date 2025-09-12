//! Event handling for the web backend.

use wasm_bindgen::prelude::*;
use web_sys::{Event, HtmlElement, KeyboardEvent, MouseEvent};

/// Event handler trait for web elements.
pub trait EventHandler {
    fn handle_event(&self, event: &Event);
}

/// Generic callback type for events.
pub type EventCallback = Box<dyn Fn(&Event)>;
pub type ClickCallback = Box<dyn Fn()>;
pub type KeyCallback = Box<dyn Fn(&KeyboardEvent)>;
pub type MouseCallback = Box<dyn Fn(&MouseEvent)>;

/// Attach an event listener to a web element.
pub fn attach_event_listener<F>(
    element: &HtmlElement,
    event_type: &str,
    handler: F,
) -> Result<(), JsValue>
where
    F: Fn(&Event) + 'static,
{
    let closure = Closure::wrap(Box::new(handler) as Box<dyn Fn(&Event)>);
    element.add_event_listener_with_callback(event_type, closure.as_ref().unchecked_ref())?;
    closure.forget(); // Keep the closure alive
    Ok(())
}

/// Attach a click event listener.
pub fn attach_click_listener<F>(element: &HtmlElement, handler: F) -> Result<(), JsValue>
where
    F: Fn() + 'static,
{
    let closure = Closure::wrap(Box::new(move |_event: Event| {
        handler();
    }) as Box<dyn Fn(Event)>);
    element
        .add_event_listener_with_callback(EventTypes::CLICK, closure.as_ref().unchecked_ref())?;
    closure.forget();
    Ok(())
}

/// Attach a keyboard event listener.
pub fn attach_keyboard_listener<F>(
    element: &HtmlElement,
    event_type: &str,
    handler: F,
) -> Result<(), JsValue>
where
    F: Fn(&KeyboardEvent) + 'static,
{
    let closure = Closure::wrap(Box::new(move |event: Event| {
        if let Some(keyboard_event) = event.dyn_ref::<KeyboardEvent>() {
            handler(keyboard_event);
        }
    }) as Box<dyn Fn(Event)>);
    element.add_event_listener_with_callback(event_type, closure.as_ref().unchecked_ref())?;
    closure.forget();
    Ok(())
}

/// Attach a mouse event listener.
pub fn attach_mouse_listener<F>(
    element: &HtmlElement,
    event_type: &str,
    handler: F,
) -> Result<(), JsValue>
where
    F: Fn(&MouseEvent) + 'static,
{
    let closure = Closure::wrap(Box::new(move |event: Event| {
        if let Some(mouse_event) = event.dyn_ref::<MouseEvent>() {
            handler(mouse_event);
        }
    }) as Box<dyn Fn(Event)>);
    element.add_event_listener_with_callback(event_type, closure.as_ref().unchecked_ref())?;
    closure.forget();
    Ok(())
}

/// Common event types for web elements.
pub struct EventTypes;

impl EventTypes {
    pub const CLICK: &'static str = "click";
    pub const CHANGE: &'static str = "change";
    pub const INPUT: &'static str = "input";
    pub const FOCUS: &'static str = "focus";
    pub const BLUR: &'static str = "blur";
    pub const KEYDOWN: &'static str = "keydown";
    pub const KEYUP: &'static str = "keyup";
    pub const KEYPRESS: &'static str = "keypress";
    pub const MOUSEOVER: &'static str = "mouseover";
    pub const MOUSEOUT: &'static str = "mouseout";
    pub const MOUSEDOWN: &'static str = "mousedown";
    pub const MOUSEUP: &'static str = "mouseup";
    pub const MOUSEMOVE: &'static str = "mousemove";
    pub const SUBMIT: &'static str = "submit";
    pub const LOAD: &'static str = "load";
    pub const ERROR: &'static str = "error";
    pub const RESIZE: &'static str = "resize";
    pub const SCROLL: &'static str = "scroll";
}
