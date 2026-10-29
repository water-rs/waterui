//! Translates cocoa-ui's platform input vocabulary into
//! `waterui-graphics`' `SurfaceInputEvent` — what `WuiGpuSurfaceInput.swift`
//! did for the Swift surface.

use waterui_graphics::input::SurfaceInputEvent;

use std::cell::Cell;

use cocoa_ui::input::{PointerButton, PointerButtons, TouchPhase};

/// Converts cocoa-ui's platform input vocabulary into the events a
/// [`GpuContentView`](waterui_graphics::gpu::GpuContentView) consumes.
///
/// The content vocabulary is a single pointer, so the translator follows the
/// first touch of a gesture as that pointer — the press it starts, the moves
/// it makes and the release that ends it — and drops the other fingers
/// until it is up. The vocabulary has no cancellation, so a cancelled touch
/// ends the press the only way it can, as a release.
#[derive(Debug, Default)]
pub struct Translator {
    /// The followed touch and the button its press reported.
    primary: Cell<Option<(u64, PointerButton)>>,
}

impl Translator {
    /// The content events `event` becomes — none, one, or a move followed
    /// by a press.
    pub fn translate(
        &self,
        event: &cocoa_ui::input::SurfaceEvent,
    ) -> impl Iterator<Item = SurfaceInputEvent> + use<> {
        use cocoa_ui::input::SurfaceEvent as E;
        let point = |position: &kurbo::Point| kurbo::Point::new(position.x, position.y);
        let (first, second) = match event {
            E::Touch {
                id,
                phase,
                position,
                buttons,
                ..
            } => match (*phase, self.primary.get()) {
                (TouchPhase::Began, None) => {
                    let button = if buttons.contains(PointerButtons::SECONDARY)
                        && !buttons.contains(PointerButtons::PRIMARY)
                    {
                        PointerButton::Secondary
                    } else {
                        PointerButton::Primary
                    };
                    self.primary.set(Some((*id, button)));
                    (
                        Some(SurfaceInputEvent::PointerMove {
                            position: point(position),
                        }),
                        Some(SurfaceInputEvent::PointerButton {
                            pressed: true,
                            button: pointer_button(button),
                            position: point(position),
                        }),
                    )
                }
                (TouchPhase::Moved, Some((primary, _))) if primary == *id => (
                    Some(SurfaceInputEvent::PointerMove {
                        position: point(position),
                    }),
                    None,
                ),
                (TouchPhase::Ended | TouchPhase::Cancelled, Some((primary, button)))
                    if primary == *id =>
                {
                    self.primary.set(None);
                    (
                        Some(SurfaceInputEvent::PointerButton {
                            pressed: false,
                            button: pointer_button(button),
                            position: point(position),
                        }),
                        None,
                    )
                }
                _ => (None, None),
            },
            E::Hover {
                position: Some(position),
                ..
            } => (
                Some(SurfaceInputEvent::PointerMove {
                    position: point(position),
                }),
                None,
            ),
            // The content vocabulary has no hover exit, pinch or rotation.
            E::Hover { position: None, .. } | E::Magnification { .. } | E::Rotation { .. } => {
                (None, None)
            }
            // The content vocabulary cannot name which keys were lost; the
            // modifier chord is the one key state it can reset.
            E::KeyboardCancel => (
                Some(SurfaceInputEvent::Modifiers(
                    cocoa_ui::input::Modifiers::empty(),
                )),
                None,
            ),
            other => (Some(Self::translate_other(other)), None),
        };
        first.into_iter().chain(second)
    }

    /// The one-to-one mappings.
    #[allow(clippy::too_many_lines)]
    fn translate_other(event: &cocoa_ui::input::SurfaceEvent) -> SurfaceInputEvent {
        use cocoa_ui::input::SurfaceEvent as E;
        use waterui_graphics::input::ScrollUnit;
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
            } => SurfaceInputEvent::PointerButton {
                pressed: *pressed,
                button: pointer_button(*button),
                position: kurbo::Point::new(position.x, position.y),
            },
            E::Scroll {
                position,
                delta_x,
                delta_y,
                unit,
                finished,
            } => SurfaceInputEvent::Scroll {
                position: kurbo::Point::new(position.x, position.y),
                delta_x: *delta_x,
                delta_y: *delta_y,
                unit: match unit {
                    cocoa_ui::input::ScrollUnit::Line => ScrollUnit::Line,
                    cocoa_ui::input::ScrollUnit::Pixel => ScrollUnit::Pixel,
                },
                finished: *finished,
            },
            // The content's scroll vocabulary is unphased: a glide's last
            // step is the one that finishes it.
            E::TrackpadPan {
                position,
                delta_x,
                delta_y,
                phase,
            } => SurfaceInputEvent::Scroll {
                position: kurbo::Point::new(position.x, position.y),
                delta_x: *delta_x,
                delta_y: *delta_y,
                unit: ScrollUnit::Pixel,
                finished: matches!(
                    phase,
                    cocoa_ui::input::GesturePhase::Ended | cocoa_ui::input::GesturePhase::Cancelled
                ),
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
            E::CompositionStart => SurfaceInputEvent::CompositionStart,
            E::CompositionUpdate { text, caret } => SurfaceInputEvent::CompositionUpdate {
                text: text.clone().into(),
                caret: *caret,
            },
            E::CompositionCommit(text) => SurfaceInputEvent::CompositionCommit(text.clone().into()),
            E::CompositionCancel => SurfaceInputEvent::CompositionCancel,
            E::Touch { .. }
            | E::Hover { .. }
            | E::Magnification { .. }
            | E::Rotation { .. }
            | E::KeyboardCancel => {
                unreachable!("translated by `Translator::translate`")
            }
        }
    }
}

const fn pointer_button(
    button: cocoa_ui::input::PointerButton,
) -> waterui_graphics::input::SurfacePointerButton {
    use waterui_graphics::input::SurfacePointerButton;
    match button {
        cocoa_ui::input::PointerButton::Primary => SurfacePointerButton::Primary,
        cocoa_ui::input::PointerButton::Secondary => SurfacePointerButton::Secondary,
        cocoa_ui::input::PointerButton::Middle => SurfacePointerButton::Middle,
        cocoa_ui::input::PointerButton::Back => SurfacePointerButton::Back,
        cocoa_ui::input::PointerButton::Forward => SurfacePointerButton::Forward,
    }
}
