//! Translates cocoa-ui's platform input vocabulary into
//! `waterui-graphics`' `SurfaceInputEvent` — what `WuiGpuSurfaceInput.swift`
//! did for the Swift surface.

use cocoa_ui::input::{GesturePhase, ScrollPhase};
use waterui_graphics::input::{self, SurfaceInputEvent};

/// Converts a [`cocoa_ui::input::SurfaceEvent`] into the event a
/// [`GpuContentView`](waterui_graphics::gpu::GpuContentView) consumes.
///
/// The kit vocabulary mirrors the platform events it translates; the mapping
/// is one-to-one except pointer buttons with no W3C meaning, which the kit
/// already declined to deliver. The kit has no resize event: the surface's
/// logical size and scale are the backend's to report, from where it learns
/// them.
#[allow(clippy::too_many_lines)]
pub fn translate(event: &cocoa_ui::input::SurfaceEvent) -> SurfaceInputEvent {
    use cocoa_ui::input::SurfaceEvent as E;
    match event {
        E::Focus(focused) => SurfaceInputEvent::Focus(*focused),
        E::Modifiers(modifiers) => SurfaceInputEvent::Modifiers(*modifiers),
        E::PointerMove { position } => SurfaceInputEvent::PointerMove {
            position: kurbo::Point::new(position.x, position.y),
        },
        E::PointerButton {
            pressed,
            button,
            position,
            click_count,
        } => SurfaceInputEvent::PointerButton {
            pressed: *pressed,
            button: pointer_button(*button),
            position: kurbo::Point::new(position.x, position.y),
            click_count: *click_count,
        },
        E::Scroll {
            position,
            delta_x,
            delta_y,
            unit,
            phase,
        } => SurfaceInputEvent::Scroll {
            position: kurbo::Point::new(position.x, position.y),
            delta_x: *delta_x,
            delta_y: *delta_y,
            unit: match unit {
                cocoa_ui::input::ScrollUnit::Line => input::ScrollUnit::Line,
                cocoa_ui::input::ScrollUnit::Pixel => input::ScrollUnit::Pixel,
            },
            phase: scroll_phase(*phase),
        },
        E::Key {
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
        E::TextInput(text) => SurfaceInputEvent::TextInput(text.clone().into()),
        E::Paste(text) => SurfaceInputEvent::Paste(text.clone().into()),
        E::CompositionStart => SurfaceInputEvent::CompositionStart,
        E::CompositionUpdate { text, caret } => SurfaceInputEvent::CompositionUpdate {
            text: text.clone().into(),
            caret: *caret,
        },
        E::CompositionCommit(text) => SurfaceInputEvent::CompositionCommit(text.clone().into()),
        E::CompositionCancel => SurfaceInputEvent::CompositionCancel,
    }
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

    use cocoa_ui::input::{GesturePhase, ScrollPhase, ScrollUnit, SurfaceEvent};
    use waterui_graphics::input::{self, SurfaceInputEvent};

    use super::translate;

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
