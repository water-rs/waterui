//! `cocoa-ui` surface events as renderer input events.
//!
//! `UIKit` coordinates are logical points in the input view's space, which is
//! the window's content space, so positions pass through without a density
//! division. Text never arrives here: the input view applies it to the
//! document client, and a view that reached this translator with a text event
//! would be bypassing that contract.

use std::collections::HashMap;

use cocoa_ui::input::{
    GesturePhase, PointerButtons, ScrollUnit, SurfaceEvent, TouchKind, TouchPhase,
};

use crate::platform::TouchPhase as RendererPhase;
use crate::platform::{InputEvent, KeyState, PointerButton, PointerKind};
use crate::runner::editing::key_code;

/// Translates one scene's surface events, remembering the button each
/// indirect-pointer press began with so its release names the same button.
#[derive(Debug, Default)]
pub(super) struct InputTranslator {
    held_buttons: HashMap<u64, PointerButton>,
}

/// The first button a mask holds; a press with an empty mask is the primary one.
fn first_button(buttons: PointerButtons) -> PointerButton {
    [
        (PointerButtons::PRIMARY, PointerButton::Primary),
        (PointerButtons::SECONDARY, PointerButton::Secondary),
        (PointerButtons::MIDDLE, PointerButton::Middle),
        (PointerButtons::BACK, PointerButton::Back),
        (PointerButtons::FORWARD, PointerButton::Forward),
    ]
    .into_iter()
    .find_map(|(flag, button)| buttons.contains(flag).then_some(button))
    .unwrap_or(PointerButton::Primary)
}

/// The id of the hovering mouse pointer, the one id `winit` gives it too. A
/// click's id is its touch's address, so the two streams never collide.
const MOUSE_HOVER_ID: u64 = 0;

/// The id of the hovering pencil.
const PEN_HOVER_ID: u64 = 1;

/// The renderer's phase for a gesture phase: the renderer calls a gesture's
/// first step `Started` and its middle `Moved`.
const fn renderer_phase(phase: GesturePhase) -> RendererPhase {
    match phase {
        GesturePhase::Began => RendererPhase::Started,
        GesturePhase::Changed => RendererPhase::Moved,
        GesturePhase::Ended => RendererPhase::Ended,
        GesturePhase::Cancelled => RendererPhase::Cancelled,
    }
}

/// The renderer event for a pointer or pencil hovering at `position`, or
/// leaving the view (`None`). Leaving cancels the stream, as `winit`'s
/// `CursorLeft` does.
fn hover_event(position: Option<kurbo::Point>, kind: TouchKind) -> InputEvent {
    let (id, kind) = match kind {
        TouchKind::IndirectPointer => (MOUSE_HOVER_ID, PointerKind::Mouse),
        TouchKind::Pencil => (PEN_HOVER_ID, PointerKind::Pen),
        TouchKind::Direct => panic!("a finger cannot hover"),
    };
    position.map_or(InputEvent::PointerCancel { id, kind }, |position| {
        InputEvent::PointerMove {
            id,
            kind,
            x: crate::num_cast::f64_as_f32(position.x),
            y: crate::num_cast::f64_as_f32(position.y),
        }
    })
}

impl InputTranslator {
    /// The renderer event for one touch of `kind` changing to `phase`.
    fn touch_event(
        &mut self,
        id: u64,
        phase: TouchPhase,
        position: kurbo::Point,
        kind: TouchKind,
        buttons: PointerButtons,
    ) -> InputEvent {
        let (x, y) = (
            crate::num_cast::f64_as_f32(position.x),
            crate::num_cast::f64_as_f32(position.y),
        );
        let (kind, button) = match kind {
            TouchKind::Direct => (PointerKind::Touch, PointerButton::Primary),
            TouchKind::Pencil => (PointerKind::Pen, PointerButton::Primary),
            TouchKind::IndirectPointer => (
                PointerKind::Mouse,
                match phase {
                    TouchPhase::Began => {
                        let pressed = first_button(buttons);
                        self.held_buttons.insert(id, pressed);
                        pressed
                    }
                    TouchPhase::Moved => PointerButton::Primary,
                    TouchPhase::Ended | TouchPhase::Cancelled => self
                        .held_buttons
                        .remove(&id)
                        .unwrap_or(PointerButton::Primary),
                },
            ),
        };
        match phase {
            TouchPhase::Began => InputEvent::PointerDown {
                id,
                kind,
                x,
                y,
                button,
            },
            TouchPhase::Moved => InputEvent::PointerMove { id, kind, x, y },
            TouchPhase::Ended => InputEvent::PointerUp {
                id,
                kind,
                x,
                y,
                button,
            },
            TouchPhase::Cancelled => InputEvent::PointerCancel { id, kind },
        }
    }

    /// The renderer events `event` stands for, in delivery order.
    ///
    /// # Panics
    /// On a text or composition event, or an `AppKit`-only pointer variant:
    /// the `UIKit` input view produces none of them.
    pub(super) fn translate(&mut self, event: SurfaceEvent) -> Option<InputEvent> {
        match event {
            SurfaceEvent::Touch {
                id,
                phase,
                position,
                kind,
                buttons,
            } => Some(self.touch_event(id, phase, position, kind, buttons)),
            SurfaceEvent::Scroll {
                position,
                delta_x,
                delta_y,
                unit,
                finished: _,
            } => Some(InputEvent::Scroll {
                x: crate::num_cast::f64_as_f32(position.x),
                y: crate::num_cast::f64_as_f32(position.y),
                // The input view reports the direction the content moves;
                // the renderer's convention is the direction the finger
                // travels.
                dx: -crate::num_cast::f64_as_f32(delta_x),
                dy: -crate::num_cast::f64_as_f32(delta_y),
                is_line_delta: matches!(unit, ScrollUnit::Line),
            }),
            // The surface reports the direction the content moves; the
            // renderer's phased trackpad scroll takes the direction the
            // fingers travel, like `winit`'s.
            SurfaceEvent::TrackpadPan {
                position,
                delta_x,
                delta_y,
                phase,
            } => Some(InputEvent::TrackpadPan {
                x: crate::num_cast::f64_as_f32(position.x),
                y: crate::num_cast::f64_as_f32(position.y),
                dx: -crate::num_cast::f64_as_f32(delta_x),
                dy: -crate::num_cast::f64_as_f32(delta_y),
                phase: renderer_phase(phase),
            }),
            SurfaceEvent::Magnification {
                position,
                delta,
                phase,
            } => Some(InputEvent::Magnification {
                x: crate::num_cast::f64_as_f32(position.x),
                y: crate::num_cast::f64_as_f32(position.y),
                delta: crate::num_cast::f64_as_f32(delta),
                phase: renderer_phase(phase),
            }),
            SurfaceEvent::Rotation {
                position,
                delta,
                phase,
            } => Some(InputEvent::Rotation {
                x: crate::num_cast::f64_as_f32(position.x),
                y: crate::num_cast::f64_as_f32(position.y),
                delta: crate::num_cast::f64_as_f32(delta),
                phase: renderer_phase(phase),
            }),
            SurfaceEvent::Hover { position, kind } => Some(hover_event(position, kind)),
            SurfaceEvent::Key {
                pressed,
                key,
                code,
                modifiers,
                repeat,
            } => Some(InputEvent::Key {
                key: key_code(&key),
                logical_key: key,
                physical_code: code,
                repeat,
                state: if pressed {
                    KeyState::Pressed
                } else {
                    KeyState::Released
                },
                modifiers: modifiers.into(),
            }),
            SurfaceEvent::Modifiers(modifiers) => {
                Some(InputEvent::ModifiersChanged(modifiers.into()))
            }
            SurfaceEvent::KeyboardCancel => Some(InputEvent::KeyboardCancel),
            // Keyboard focus of the input view follows the soft keyboard; the
            // window's own focus is the scene's activation, reported by the
            // lifecycle handlers.
            SurfaceEvent::Focus(_) => None,
            SurfaceEvent::PointerMove { .. }
            | SurfaceEvent::PointerButton { .. }
            | SurfaceEvent::TextInput(_)
            | SurfaceEvent::CompositionStart
            | SurfaceEvent::CompositionUpdate { .. }
            | SurfaceEvent::CompositionCommit(_)
            | SurfaceEvent::CompositionCancel => {
                panic!("the UIKit input view delivered {event:?}, which it never produces")
            }
        }
    }
}
