//! Material Design 3 vertical drag handle composed from `WaterUI` primitives.
//!
//! This is the grip on a resizable pane or sheet, not the reorder grip a list
//! row shows. It is a slim bar at rest that thickens and darkens while it is
//! being held, so the thing under the finger is unmistakably the thing that
//! moves.

use waterui::gesture::{DragEvent, DragGesture, GesturePhase};
use waterui::layout::frame::Frame;
use waterui::reactive::{SignalExt as _, binding};
use waterui::shape::{Capsule, ShapeExt as _};
use waterui::{Environment, Str, View, ViewExt as _};
use waterui_core::handler::{Handler, boxed_action};

use crate::color::{OnSurface, Outline};
use crate::semantics::conditional_color;

/// `DragHandleTokens.Width`.
const WIDTH: f32 = 4.0;
/// `DragHandleTokens.Height`.
const HEIGHT: f32 = 48.0;
/// `DragHandleTokens.PressedWidth`, shared with the dragged state.
const PRESSED_WIDTH: f32 = 12.0;
/// `DragHandleTokens.PressedHeight`, shared with the dragged state.
const PRESSED_HEIGHT: f32 = 52.0;
/// `DragHandleTokens.ContainerWidth`: the hit area around the bar, which stays
/// this wide whatever the bar itself is doing.
const CONTAINER_WIDTH: f32 = 24.0;

/// A Material Design 3 vertical drag handle.
///
/// `DragHandleTokens` gives the pressed and dragged states identical geometry,
/// so one `held` flag covers both.
#[derive(Debug)]
pub struct VerticalDragHandle<Action = fn(&Environment)> {
    accessibility_label: Str,
    on_drag: Action,
}

impl VerticalDragHandle {
    /// Creates a vertical drag handle.
    ///
    /// The handle has no visible text, so `accessibility_label` is what names
    /// the thing it resizes.
    #[must_use]
    pub fn new(accessibility_label: impl Into<Str>) -> Self {
        Self {
            accessibility_label: accessibility_label.into(),
            on_drag: noop,
        }
    }
}

impl<Action> VerticalDragHandle<Action> {
    /// Sets the handler run as the handle is dragged.
    ///
    /// The handler receives the drag through the environment, so it can read
    /// the translation and phase and resize whatever the handle controls.
    #[must_use]
    pub fn on_drag<F, Args>(self, action: F) -> VerticalDragHandle<impl FnMut(&Environment)>
    where
        F: Handler<Args, ()> + 'static,
    {
        VerticalDragHandle {
            accessibility_label: self.accessibility_label,
            on_drag: boxed_action(action),
        }
    }
}

impl<Action> View for VerticalDragHandle<Action>
where
    Action: FnMut(&Environment) + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        let mut on_drag = self.on_drag;
        // Held covers both the pressed and dragged states, which share their
        // geometry and colour.
        let held = binding(false);
        let held_for_gesture = held.clone();

        let width = held.map(|held| if held { PRESSED_WIDTH } else { WIDTH });
        let height = held.map(|held| if held { PRESSED_HEIGHT } else { HEIGHT });
        // `DragHandleTokens.Color` is Outline; pressed and dragged are OnSurface.
        let color = conditional_color(held, OnSurface, Outline);

        // `ViewExt::width` is a fixed size; the bar's own dimensions are
        // reactive, so the inner frame takes signals directly.
        Frame::new(Capsule.fill(color))
            .width(width)
            .height(height)
            .width(CONTAINER_WIDTH)
            .gesture(DragGesture::new(0.0), move |env: Environment| {
                let phase = env
                    .get::<DragEvent>()
                    .expect("drag handle gesture is missing its DragEvent")
                    .phase;
                held_for_gesture.set(matches!(
                    phase,
                    GesturePhase::Started | GesturePhase::Updated
                ));
                on_drag(&env);
            })
            .a11y_label(self.accessibility_label)
    }
}

const fn noop(_env: &Environment) {}

/// Creates a Material Design 3 vertical drag handle.
#[must_use]
pub fn vertical_drag_handle(accessibility_label: impl Into<Str>) -> VerticalDragHandle {
    VerticalDragHandle::new(accessibility_label)
}

#[cfg(test)]
mod tests {
    use super::{CONTAINER_WIDTH, HEIGHT, PRESSED_HEIGHT, PRESSED_WIDTH, WIDTH};

    /// Values from `androidx.compose.material3.tokens.DragHandleTokens`.
    #[test]
    fn drag_handle_tokens_match_compose_drag_handle_tokens() {
        assert_eq!(WIDTH, 4.0);
        assert_eq!(HEIGHT, 48.0);
        assert_eq!(PRESSED_WIDTH, 12.0);
        assert_eq!(PRESSED_HEIGHT, 52.0);
        assert_eq!(CONTAINER_WIDTH, 24.0);
    }

    /// The bar grows in both axes while held, and never outgrows the container
    /// that catches the pointer. These hold at compile time, so a token edit
    /// that broke them would fail the build rather than a test run.
    #[test]
    fn holding_the_handle_thickens_it_within_its_hit_area() {
        const {
            assert!(PRESSED_WIDTH > WIDTH);
            assert!(PRESSED_HEIGHT > HEIGHT);
            assert!(
                PRESSED_WIDTH <= CONTAINER_WIDTH,
                "the bar must stay inside the hit area it grows within"
            );
        }
    }
}
