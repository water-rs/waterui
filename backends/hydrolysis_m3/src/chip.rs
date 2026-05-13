//! Material Design 3 chip components composed from WaterUI primitives.

use core::fmt::{self, Debug};
use core::marker::PhantomData;
use waterui::accessibility::{AccessibilityChildren, AccessibilityRole, AccessibilityState};
use waterui::border::Border;
use waterui::color::Color;
use waterui::layout::padding::EdgeInsets;
use waterui::layout::Point;
use waterui::reactive::SignalExt as _;
use waterui::shape::{RoundedRectangle, ShapeExt as _};
use waterui::widget::condition::when;
use waterui::{Binding, Environment, Signal, Str, View, ViewExt as _};
use waterui_canvas::{Canvas, DrawingContext, LineCap, LineJoin};
use waterui_core::handler::{Handler, SharedAction, boxed_action};
use waterui_core::resolve::Resolvable as _;
use waterui_controls::label::{IntoLabel, Label};

use crate::color::{
    OnSecondaryContainer, OnSurface, OnSurfaceVariant, Outline, SecondaryContainer, Surface,
};
use crate::theme::typography;

const ASSIST_CHIP_CONTAINER_HEIGHT: f32 = 32.0;
const ASSIST_CHIP_CONTAINER_SHAPE: f32 = 8.0;
const ASSIST_CHIP_CONTAINER_CLIP_RADIUS: f32 = 0.25;
const ASSIST_CHIP_OUTLINE_WIDTH: f32 = 1.0;
const ASSIST_CHIP_LEADING_SPACE: f32 = 16.0;
const ASSIST_CHIP_TRAILING_SPACE: f32 = 16.0;
const FILTER_CHIP_CONTAINER_HEIGHT: f32 = 32.0;
const FILTER_CHIP_CONTAINER_SHAPE: f32 = 8.0;
const FILTER_CHIP_CONTAINER_CLIP_RADIUS: f32 = 0.25;
const FILTER_CHIP_UNSELECTED_OUTLINE_WIDTH: f32 = 1.0;
const FILTER_CHIP_SELECTED_OUTLINE_WIDTH: f32 = 0.0;
const FILTER_CHIP_LEADING_SPACE: f32 = 16.0;
const FILTER_CHIP_TRAILING_SPACE: f32 = 16.0;
const FILTER_CHIP_WITH_ICON_LEADING_SPACE: f32 = 8.0;
const FILTER_CHIP_ICON_LABEL_SPACE: f32 = 8.0;
const FILTER_CHIP_ICON_SIZE: f32 = 18.0;
const FILTER_CHIP_CHECKMARK_LINE_WIDTH: f32 = 2.0;

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

/// A Material Design 3 filter chip.
///
/// Filter chips toggle a selected state and expose that state through the
/// accessibility tree.
pub struct FilterChip<Action = fn(&Environment)> {
    label: Label,
    accessibility_label: Str,
    selected: Binding<bool>,
    action: Action,
}

impl<Action> Debug for FilterChip<Action> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FilterChip")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl FilterChip<fn(&Environment)> {
    /// Creates a filter chip bound to the provided selected state.
    #[must_use]
    pub fn new(label: impl IntoLabel, selected: &Binding<bool>) -> Self {
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
            selected: selected.clone(),
            action: noop,
        }
    }
}

impl<Action> FilterChip<Action> {
    /// Sets the action performed after the chip toggles its selected state.
    #[must_use]
    pub fn action<F, Args>(self, action: F) -> FilterChip<impl FnMut(&Environment)>
    where
        F: Handler<Args, ()> + 'static,
    {
        FilterChip {
            label: self.label,
            accessibility_label: self.accessibility_label,
            selected: self.selected,
            action: boxed_action(action),
        }
    }
}

impl<Action> View for FilterChip<Action>
where
    Action: FnMut(&Environment) + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        let mut action = self.action;
        let action = SharedAction::new(move |env: Environment| action(&env));
        let selected_for_state = self.selected.clone();
        let selected_for_tap = self.selected.clone();
        let accessibility_state =
            self.selected
                .clone()
                .map(|selected| AccessibilityState::new().selected(selected));
        let selected_label = self.label.clone();
        let unselected_label = self.label.clone();
        let selected_accessibility_label = self.accessibility_label.clone();
        let unselected_accessibility_label = self.accessibility_label;
        let selected_action = action.clone();
        let unselected_action = action;
        let selected_tap_state = selected_for_tap.clone();
        let unselected_tap_state = selected_for_tap;

        when(
            selected_for_state,
            move || {
                selected_filter_chip_view(
                    selected_label.clone(),
                    selected_accessibility_label.clone(),
                    selected_tap_state.clone(),
                    selected_action.clone(),
                )
            },
        )
        .otherwise(move || {
            unselected_filter_chip_view(
                unselected_label.clone(),
                unselected_accessibility_label.clone(),
                unselected_tap_state.clone(),
                unselected_action.clone(),
            )
        })
        .a11y_state_signal(accessibility_state)
    }
}

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

/// Creates a Material Design 3 filter chip with the given semantic label.
#[must_use]
pub fn filter_chip(label: impl IntoLabel, selected: &Binding<bool>) -> FilterChip {
    FilterChip::new(label, selected)
}

fn selected_filter_chip_view(
    label: Label,
    accessibility_label: Str,
    selected: Binding<bool>,
    action: SharedAction,
) -> impl View {
    waterui::component::hstack((
        CheckmarkIcon,
        label
            .font(typography::label_large())
            .foreground(OnSecondaryContainer),
    ))
    .spacing(FILTER_CHIP_ICON_LABEL_SPACE)
    .height(FILTER_CHIP_CONTAINER_HEIGHT)
    .padding_with(EdgeInsets::new(
        0.0,
        0.0,
        FILTER_CHIP_WITH_ICON_LEADING_SPACE,
        FILTER_CHIP_TRAILING_SPACE,
    ))
    .background(RoundedRectangle::new(FILTER_CHIP_CONTAINER_CLIP_RADIUS).fill(SecondaryContainer))
    .border_with(
        Border::new(Outline, FILTER_CHIP_SELECTED_OUTLINE_WIDTH)
            .corner_radius(FILTER_CHIP_CONTAINER_SHAPE),
    )
    .on_tap(move |env: Environment| {
        selected.set(!selected.get());
        action.call(&env);
    })
    .a11y_label(accessibility_label)
    .a11y_role(AccessibilityRole::Button)
    .a11y_children(AccessibilityChildren::ExcludeDescendants)
}

fn unselected_filter_chip_view(
    label: Label,
    accessibility_label: Str,
    selected: Binding<bool>,
    action: SharedAction,
) -> impl View {
    label
        .font(typography::label_large())
        .foreground(OnSurfaceVariant)
        .height(FILTER_CHIP_CONTAINER_HEIGHT)
        .padding_with(EdgeInsets::new(
            0.0,
            0.0,
            FILTER_CHIP_LEADING_SPACE,
            FILTER_CHIP_TRAILING_SPACE,
        ))
        .background(RoundedRectangle::new(FILTER_CHIP_CONTAINER_CLIP_RADIUS).fill(Surface))
        .border_with(
            Border::new(Outline, FILTER_CHIP_UNSELECTED_OUTLINE_WIDTH)
                .corner_radius(FILTER_CHIP_CONTAINER_SHAPE),
        )
        .on_tap(move |env: Environment| {
            selected.set(!selected.get());
            action.call(&env);
        })
        .a11y_label(accessibility_label)
        .a11y_role(AccessibilityRole::Button)
        .a11y_children(AccessibilityChildren::ExcludeDescendants)
}

struct CheckmarkIcon;

impl View for CheckmarkIcon {
    fn body(self, env: &Environment) -> impl View {
        let stroke = OnSecondaryContainer.resolve(env);
        Canvas::with_signal(stroke, |ctx: &mut DrawingContext<'_>, stroke| {
            let mut path = ctx.begin_path();
            path.move_to(Point::new(4.25, 9.25));
            path.line_to(Point::new(7.25, 12.25));
            path.line_to(Point::new(14.0, 5.5));
            ctx.set_stroke_style(stroke);
            ctx.set_line_width(FILTER_CHIP_CHECKMARK_LINE_WIDTH);
            ctx.set_line_cap(LineCap::Round);
            ctx.set_line_join(LineJoin::Round);
            ctx.stroke_path(&path);
        })
        .size(FILTER_CHIP_ICON_SIZE, FILTER_CHIP_ICON_SIZE)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ASSIST_CHIP_CONTAINER_HEIGHT, ASSIST_CHIP_CONTAINER_SHAPE, ASSIST_CHIP_LEADING_SPACE,
        ASSIST_CHIP_OUTLINE_WIDTH, ASSIST_CHIP_TRAILING_SPACE, FILTER_CHIP_CONTAINER_HEIGHT,
        FILTER_CHIP_CONTAINER_SHAPE, FILTER_CHIP_ICON_LABEL_SPACE, FILTER_CHIP_ICON_SIZE,
        FILTER_CHIP_LEADING_SPACE, FILTER_CHIP_SELECTED_OUTLINE_WIDTH,
        FILTER_CHIP_TRAILING_SPACE, FILTER_CHIP_UNSELECTED_OUTLINE_WIDTH,
        FILTER_CHIP_WITH_ICON_LEADING_SPACE,
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

    #[test]
    fn filter_chip_tokens_match_material_web_v0_192() {
        assert_eq!(FILTER_CHIP_CONTAINER_HEIGHT, 32.0);
        assert_eq!(FILTER_CHIP_CONTAINER_SHAPE, 8.0);
        assert_eq!(FILTER_CHIP_UNSELECTED_OUTLINE_WIDTH, 1.0);
        assert_eq!(FILTER_CHIP_SELECTED_OUTLINE_WIDTH, 0.0);
        assert_eq!(FILTER_CHIP_LEADING_SPACE, 16.0);
        assert_eq!(FILTER_CHIP_WITH_ICON_LEADING_SPACE, 8.0);
        assert_eq!(FILTER_CHIP_ICON_LABEL_SPACE, 8.0);
        assert_eq!(FILTER_CHIP_TRAILING_SPACE, 16.0);
        assert_eq!(FILTER_CHIP_ICON_SIZE, 18.0);
    }
}
