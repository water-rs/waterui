//! Translates cocoa-ui's platform input vocabulary into
//! `waterui-graphics`' `SurfaceInputEvent` — what `WuiGpuSurfaceInput.swift`
//! did for the Swift surface.

use std::cell::Cell;
use std::num::NonZeroU32;

use cocoa_ui::input::{
    GesturePhase, PointerButton, PointerButtons, ScrollPhase, SurfaceEvent, TouchPhase,
};
use waterui_graphics::input::{self, SurfaceInputEvent};

/// Converts cocoa-ui's platform input vocabulary into the events a
/// [`GpuView`](waterui_graphics::gpu::GpuView) consumes.
///
/// The content vocabulary is a single pointer, so the translator follows the
/// first touch of a gesture as that pointer — the press it starts, the moves
/// it makes and the release that ends it — and drops the other fingers
/// until it is up. The vocabulary has no cancellation, so a cancelled touch
/// ends the press the only way it can, as a release.
#[derive(Debug, Default)]
pub struct Translator {
    /// The followed touch, the button its press reported and the tap count
    /// of that press.
    primary: Cell<Option<(u64, PointerButton, NonZeroU32)>>,
}

impl Translator {
    /// The content events `event` becomes — none, one, or a move followed
    /// by a press.
    pub fn translate(
        &self,
        event: &SurfaceEvent,
    ) -> impl Iterator<Item = SurfaceInputEvent> + use<> {
        let (first, second) = match event {
            SurfaceEvent::Touch {
                id,
                phase,
                position,
                buttons,
                tap_count,
                ..
            } => self.touch(*id, *phase, *position, *buttons, *tap_count),
            other => (one_to_one(other), None),
        };
        first.into_iter().chain(second)
    }

    /// The pointer events one touch's phase becomes for the followed touch;
    /// every other finger is dropped.
    fn touch(
        &self,
        id: u64,
        phase: TouchPhase,
        position: kurbo::Point,
        buttons: PointerButtons,
        tap_count: NonZeroU32,
    ) -> (Option<SurfaceInputEvent>, Option<SurfaceInputEvent>) {
        match (phase, self.primary.get()) {
            (TouchPhase::Began, None) => {
                let button = if buttons.contains(PointerButtons::SECONDARY)
                    && !buttons.contains(PointerButtons::PRIMARY)
                {
                    PointerButton::Secondary
                } else {
                    PointerButton::Primary
                };
                self.primary.set(Some((id, button, tap_count)));
                (
                    Some(SurfaceInputEvent::PointerMove { position }),
                    Some(SurfaceInputEvent::PointerButton {
                        pressed: true,
                        button: pointer_button(button),
                        position,
                        click_count: tap_count,
                    }),
                )
            }
            (TouchPhase::Moved, Some((primary, _, _))) if primary == id => {
                (Some(SurfaceInputEvent::PointerMove { position }), None)
            }
            (TouchPhase::Ended | TouchPhase::Cancelled, Some((primary, button, count)))
                if primary == id =>
            {
                self.primary.set(None);
                (
                    Some(SurfaceInputEvent::PointerButton {
                        pressed: false,
                        button: pointer_button(button),
                        position,
                        click_count: count,
                    }),
                    None,
                )
            }
            _ => (None, None),
        }
    }
}

/// The content event a kit event maps onto, or `None` for the events the
/// content vocabulary has no word for — a hover exit, a pinch, a rotation —
/// and the touches [`Translator::translate`] follows itself.
fn one_to_one(event: &SurfaceEvent) -> Option<SurfaceInputEvent> {
    Some(match event {
        SurfaceEvent::Focus(focused) => SurfaceInputEvent::Focus(*focused),
        SurfaceEvent::Modifiers(modifiers) => SurfaceInputEvent::Modifiers(*modifiers),
        // The content vocabulary cannot name which keys were lost; the
        // modifier chord is the one key state it can reset.
        SurfaceEvent::KeyboardCancel => {
            SurfaceInputEvent::Modifiers(cocoa_ui::input::Modifiers::empty())
        }
        SurfaceEvent::PointerMove { position }
        | SurfaceEvent::Hover {
            position: Some(position),
            ..
        } => SurfaceInputEvent::PointerMove {
            position: *position,
        },
        SurfaceEvent::PointerButton {
            pressed,
            button,
            position,
            click_count,
        } => SurfaceInputEvent::PointerButton {
            pressed: *pressed,
            button: pointer_button(*button),
            position: *position,
            click_count: *click_count,
        },
        SurfaceEvent::Scroll {
            position,
            delta_x,
            delta_y,
            unit,
            phase,
        } => SurfaceInputEvent::Scroll {
            position: *position,
            delta_x: *delta_x,
            delta_y: *delta_y,
            unit: match unit {
                cocoa_ui::input::ScrollUnit::Line => input::ScrollUnit::Line,
                cocoa_ui::input::ScrollUnit::Pixel => input::ScrollUnit::Pixel,
            },
            phase: scroll_phase(*phase),
        },
        SurfaceEvent::Key {
            pressed,
            key,
            code,
            modifiers,
            repeat,
        } => SurfaceInputEvent::Key {
            pressed: *pressed,
            key: key.clone(),
            code: *code,
            modifiers: *modifiers,
            repeat: *repeat,
        },
        SurfaceEvent::TextInput(text) => SurfaceInputEvent::TextInput(text.clone().into()),
        SurfaceEvent::Paste(text) => SurfaceInputEvent::Paste(text.clone().into()),
        SurfaceEvent::CompositionStart => SurfaceInputEvent::CompositionStart,
        SurfaceEvent::CompositionUpdate { text, caret } => SurfaceInputEvent::CompositionUpdate {
            text: text.clone().into(),
            caret: *caret,
        },
        SurfaceEvent::CompositionCommit(text) => {
            SurfaceInputEvent::CompositionCommit(text.clone().into())
        }
        SurfaceEvent::CompositionCancel => SurfaceInputEvent::CompositionCancel,
        SurfaceEvent::Touch { .. }
        | SurfaceEvent::Hover { position: None, .. }
        | SurfaceEvent::Magnification { .. }
        | SurfaceEvent::Rotation { .. } => return None,
    })
}

const fn pointer_button(button: cocoa_ui::input::PointerButton) -> input::SurfacePointerButton {
    use input::SurfacePointerButton;
    match button {
        cocoa_ui::input::PointerButton::Primary => SurfacePointerButton::Primary,
        cocoa_ui::input::PointerButton::Secondary => SurfacePointerButton::Secondary,
        cocoa_ui::input::PointerButton::Middle => SurfacePointerButton::Middle,
        cocoa_ui::input::PointerButton::Back => SurfacePointerButton::Back,
        cocoa_ui::input::PointerButton::Forward => SurfacePointerButton::Forward,
    }
}

const fn scroll_phase(phase: ScrollPhase) -> input::ScrollPhase {
    match phase {
        ScrollPhase::Discrete => input::ScrollPhase::Discrete,
        ScrollPhase::Contact(gesture) => input::ScrollPhase::Contact(gesture_phase(gesture)),
        ScrollPhase::Momentum(gesture) => input::ScrollPhase::Momentum(gesture_phase(gesture)),
    }
}

const fn gesture_phase(phase: GesturePhase) -> input::GesturePhase {
    match phase {
        GesturePhase::Began => input::GesturePhase::Started,
        GesturePhase::Changed => input::GesturePhase::Moved,
        GesturePhase::Ended => input::GesturePhase::Ended,
        GesturePhase::Cancelled => input::GesturePhase::Cancelled,
    }
}

#[cfg(test)]
mod tests {
    use core::num::NonZeroU32;

    use cocoa_ui::input::{
        GesturePhase, PointerButtons, ScrollPhase, ScrollUnit, SurfaceEvent, TouchKind, TouchPhase,
    };
    use waterui_graphics::input::{self, SurfaceInputEvent};

    use super::Translator;

    /// The one event `event` translates to.
    fn translate(event: &SurfaceEvent) -> SurfaceInputEvent {
        let mut events = Translator::default().translate(event);
        let first = events.next().expect("the event translates to one event");
        assert!(events.next().is_none(), "the event translates to one event");
        first
    }

    #[test]
    fn a_pointer_button_keeps_its_click_count_on_press_and_release() {
        for (pressed, count) in [(true, 3), (false, 3)] {
            let event = translate(&SurfaceEvent::PointerButton {
                pressed,
                button: cocoa_ui::input::PointerButton::Primary,
                position: kurbo::Point::new(4.0, 5.0),
                click_count: NonZeroU32::new(count).expect("non-zero"),
            });
            let SurfaceInputEvent::PointerButton {
                pressed: translated,
                click_count,
                ..
            } = event
            else {
                panic!("a pointer button translates to a pointer button");
            };
            assert_eq!(translated, pressed);
            assert_eq!(click_count.get(), count);
        }
    }

    #[test]
    fn a_touch_press_and_release_carry_its_tap_count() {
        let translator = Translator::default();
        let touch = |phase| SurfaceEvent::Touch {
            id: 7,
            phase,
            position: kurbo::Point::new(1.0, 2.0),
            kind: TouchKind::Direct,
            buttons: PointerButtons::empty(),
            tap_count: NonZeroU32::new(2).expect("non-zero"),
        };
        let counts: Vec<_> = [TouchPhase::Began, TouchPhase::Ended]
            .into_iter()
            .flat_map(|phase| translator.translate(&touch(phase)).collect::<Vec<_>>())
            .filter_map(|event| match event {
                SurfaceInputEvent::PointerButton {
                    pressed,
                    click_count,
                    ..
                } => Some((pressed, click_count.get())),
                _ => None,
            })
            .collect();
        assert_eq!(counts, [(true, 2), (false, 2)]);
    }

    #[test]
    fn scroll_phases_map_gesture_names_onto_the_neutral_lifecycle() {
        let phase = |phase| {
            let SurfaceInputEvent::Scroll { phase, .. } = translate(&SurfaceEvent::Scroll {
                position: kurbo::Point::new(0.0, 0.0),
                delta_x: 0.0,
                delta_y: 0.0,
                unit: ScrollUnit::Pixel,
                phase,
            }) else {
                panic!("a scroll translates to a scroll");
            };
            phase
        };
        assert_eq!(phase(ScrollPhase::Discrete), input::ScrollPhase::Discrete);
        assert_eq!(
            phase(ScrollPhase::Contact(GesturePhase::Began)),
            input::ScrollPhase::Contact(input::GesturePhase::Started)
        );
        assert_eq!(
            phase(ScrollPhase::Contact(GesturePhase::Changed)),
            input::ScrollPhase::Contact(input::GesturePhase::Moved)
        );
        assert_eq!(
            phase(ScrollPhase::Momentum(GesturePhase::Ended)),
            input::ScrollPhase::Momentum(input::GesturePhase::Ended)
        );
        assert_eq!(
            phase(ScrollPhase::Momentum(GesturePhase::Cancelled)),
            input::ScrollPhase::Momentum(input::GesturePhase::Cancelled)
        );
    }

    #[test]
    fn a_paste_keeps_its_text() {
        assert_eq!(
            translate(&SurfaceEvent::Paste("a\nb".into())),
            SurfaceInputEvent::Paste("a\nb".into())
        );
    }
}
