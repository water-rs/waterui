//! Material Design 3 chip components composed from WaterUI primitives.

use core::fmt::{self, Debug};
use core::marker::PhantomData;
use waterui::accessibility::{AccessibilityChildren, AccessibilityRole};
use waterui::border::Border;
use waterui::color::Color;
use waterui::layout::padding::EdgeInsets;
use waterui::shape::{RoundedRectangle, ShapeExt as _};
use waterui::{Environment, Signal, Str, View, ViewExt as _};
use waterui_controls::label::{IntoLabel, Label};
use waterui_core::handler::{Handler, boxed_action};

use crate::color::{OnSurface, OnSurfaceVariant, Outline, Surface};
use crate::theme::typography;

const ASSIST_CHIP_CONTAINER_HEIGHT: f32 = 32.0;
const ASSIST_CHIP_CONTAINER_SHAPE: f32 = 8.0;
const ASSIST_CHIP_CONTAINER_CLIP_RADIUS: f32 = 0.25;
const ASSIST_CHIP_OUTLINE_WIDTH: f32 = 1.0;
const ASSIST_CHIP_LEADING_SPACE: f32 = 16.0;
const ASSIST_CHIP_TRAILING_SPACE: f32 = 16.0;

/// Shared Material Design 3 outlined action chip foundation.
///
/// This implementation is pure WaterUI composition: no Hydrolysis renderer type
/// or backend-specific view ID is introduced.
pub struct OutlinedChip<Action = fn(&Environment), LabelColor = OnSurface> {
    label: Label,
    accessibility_label: Str,
    action: Action,
    label_color: PhantomData<LabelColor>,
}

impl<Action, LabelColor> Debug for OutlinedChip<Action, LabelColor> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutlinedChip")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl<LabelColor> OutlinedChip<fn(&Environment), LabelColor> {
    /// Creates an outlined chip with the given semantic label.
    #[must_use]
    pub fn new(label: impl IntoLabel) -> Self {
        let label = label.into_label();
        let accessibility_label = label
            .semantic_text()
            .clone()
            .resolve(&Environment::new())
            .content
            .get()
            .to_plain();
        Self {
            label,
            accessibility_label,
            action: noop,
            label_color: PhantomData,
        }
    }
}

impl<Action, LabelColor> OutlinedChip<Action, LabelColor> {
    /// Sets the action performed when the chip is tapped.
    #[must_use]
    pub fn action<F, Args>(self, action: F) -> OutlinedChip<impl FnMut(&Environment), LabelColor>
    where
        F: Handler<Args, ()> + 'static,
    {
        OutlinedChip {
            label: self.label,
            accessibility_label: self.accessibility_label,
            action: boxed_action(action),
            label_color: PhantomData,
        }
    }
}

impl<Action, LabelColor> View for OutlinedChip<Action, LabelColor>
where
    Action: FnMut(&Environment) + 'static,
    LabelColor: Default + Into<Color> + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        let mut action = self.action;
        let accessibility_label = self.accessibility_label.clone();
        let label = self
            .label
            .clone()
            .font(typography::label_large())
            .foreground(LabelColor::default());

        label
            .height(ASSIST_CHIP_CONTAINER_HEIGHT)
            .padding_with(EdgeInsets::new(
                0.0,
                0.0,
                ASSIST_CHIP_LEADING_SPACE,
                ASSIST_CHIP_TRAILING_SPACE,
            ))
            .background(RoundedRectangle::new(ASSIST_CHIP_CONTAINER_CLIP_RADIUS).fill(Surface))
            .border_with(
                Border::new(Outline, ASSIST_CHIP_OUTLINE_WIDTH)
                    .corner_radius(ASSIST_CHIP_CONTAINER_SHAPE),
            )
            .on_tap(move |env: Environment| action(&env))
            .a11y_label(accessibility_label)
            .a11y_role(AccessibilityRole::Button)
            .a11y_children(AccessibilityChildren::ExcludeDescendants)
    }
}

const fn noop(_env: &Environment) {}

/// A Material Design 3 assist chip.
///
/// Assist chips represent a smart or supplemental action related to nearby
/// content.
pub type AssistChip<Action = fn(&Environment)> = OutlinedChip<Action, OnSurface>;

/// A Material Design 3 suggestion chip.
///
/// Suggestion chips present quick suggestions related to user input or content.
pub type SuggestionChip<Action = fn(&Environment)> = OutlinedChip<Action, OnSurfaceVariant>;

/// Creates a Material Design 3 assist chip with the given semantic label.
#[must_use]
pub fn assist_chip(label: impl IntoLabel) -> AssistChip {
    AssistChip::new(label)
}

/// Creates a Material Design 3 suggestion chip with the given semantic label.
#[must_use]
pub fn suggestion_chip(label: impl IntoLabel) -> SuggestionChip {
    SuggestionChip::new(label)
}

#[cfg(test)]
mod tests {
    use super::{
        ASSIST_CHIP_CONTAINER_HEIGHT, ASSIST_CHIP_CONTAINER_SHAPE, ASSIST_CHIP_LEADING_SPACE,
        ASSIST_CHIP_OUTLINE_WIDTH, ASSIST_CHIP_TRAILING_SPACE,
    };

    #[test]
    fn assist_chip_tokens_match_material_web_v0_192() {
        assert_eq!(ASSIST_CHIP_CONTAINER_HEIGHT, 32.0);
        assert_eq!(ASSIST_CHIP_CONTAINER_SHAPE, 8.0);
        assert_eq!(ASSIST_CHIP_OUTLINE_WIDTH, 1.0);
        assert_eq!(ASSIST_CHIP_LEADING_SPACE, 16.0);
        assert_eq!(ASSIST_CHIP_TRAILING_SPACE, 16.0);
    }

    #[test]
    fn suggestion_chip_uses_same_outline_geometry_as_assist_chip() {
        let _chip = crate::suggestion_chip("Suggestion");

        assert_eq!(ASSIST_CHIP_CONTAINER_HEIGHT, 32.0);
        assert_eq!(ASSIST_CHIP_CONTAINER_SHAPE, 8.0);
        assert_eq!(ASSIST_CHIP_OUTLINE_WIDTH, 1.0);
    }
}
