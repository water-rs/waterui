//! Material Design 3 chip components composed from WaterUI primitives.

use core::fmt::{self, Debug};
use core::marker::PhantomData;
use waterui::accessibility::{AccessibilityChildren, AccessibilityRole, AccessibilityState};
use waterui::border::Border;
use waterui::color::Color;
use waterui::layout::padding::EdgeInsets;
use waterui::reactive::SignalExt as _;
use waterui::shape::{Rectangle, RoundedRectangle, ShapeExt as _};
use waterui::widget::condition::when;
use waterui::{Binding, Environment, Signal, Str, View, ViewExt as _};
use waterui_controls::label::{IntoLabel, Label};
use waterui_core::handler::{Handler, SharedAction, boxed_action};

use crate::color::{
    OnSecondaryContainer, OnSurface, OnSurfaceVariant, Outline, SecondaryContainer, Surface,
};
use crate::icons::CheckmarkIcon;
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
const INPUT_CHIP_CONTAINER_HEIGHT: f32 = 32.0;
const INPUT_CHIP_CONTAINER_SHAPE: f32 = 8.0;
const INPUT_CHIP_CONTAINER_CLIP_RADIUS: f32 = 0.25;
const INPUT_CHIP_UNSELECTED_OUTLINE_WIDTH: f32 = 1.0;
const INPUT_CHIP_LEADING_SPACE: f32 = 16.0;
const INPUT_CHIP_WITH_TRAILING_ICON_TRAILING_SPACE: f32 = 8.0;
const INPUT_CHIP_ICON_LABEL_SPACE: f32 = 8.0;
const INPUT_CHIP_TRAILING_ICON_SIZE: f32 = 18.0;
const INPUT_CHIP_REMOVE_ICON_LINE_WIDTH: f32 = 2.0;

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

/// A Material Design 3 input chip with a trailing remove action.
pub struct InputChip<Action = fn(&Environment), RemoveAction = fn(&Environment)> {
    label: Label,
    accessibility_label: Str,
    remove_accessibility_label: Str,
    action: Action,
    remove_action: RemoveAction,
}

impl<Action, RemoveAction> Debug for InputChip<Action, RemoveAction> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InputChip")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl InputChip<fn(&Environment), fn(&Environment)> {
    /// Creates an input chip with the given semantic label.
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
        let remove_accessibility_label = Str::from(format!("Remove {accessibility_label}"));
        Self {
            label,
            accessibility_label,
            remove_accessibility_label,
            action: noop,
            remove_action: noop,
        }
    }
}

impl<Action, RemoveAction> InputChip<Action, RemoveAction> {
    /// Sets the action performed when the primary chip area is tapped.
    #[must_use]
    pub fn action<F, Args>(self, action: F) -> InputChip<impl FnMut(&Environment), RemoveAction>
    where
        F: Handler<Args, ()> + 'static,
    {
        InputChip {
            label: self.label,
            accessibility_label: self.accessibility_label,
            remove_accessibility_label: self.remove_accessibility_label,
            action: boxed_action(action),
            remove_action: self.remove_action,
        }
    }

    /// Sets the action performed when the trailing remove button is tapped.
    #[must_use]
    pub fn remove_action<F, Args>(
        self,
        action: F,
    ) -> InputChip<Action, impl FnMut(&Environment)>
    where
        F: Handler<Args, ()> + 'static,
    {
        InputChip {
            label: self.label,
            accessibility_label: self.accessibility_label,
            remove_accessibility_label: self.remove_accessibility_label,
            action: self.action,
            remove_action: boxed_action(action),
        }
    }
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

impl<Action, RemoveAction> View for InputChip<Action, RemoveAction>
where
    Action: FnMut(&Environment) + 'static,
    RemoveAction: FnMut(&Environment) + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        let mut action = self.action;
        let action = SharedAction::new(move |env: Environment| action(&env));
        let mut remove_action = self.remove_action;
        let remove_action = SharedAction::new(move |env: Environment| remove_action(&env));
        let accessibility_label = self.accessibility_label;
        let remove_accessibility_label = self.remove_accessibility_label;

        waterui::component::hstack((
            self.label
                .font(typography::label_large())
                .foreground(OnSurfaceVariant)
                .on_tap(move |env: Environment| action.call(&env))
                .a11y_label(accessibility_label)
                .a11y_role(AccessibilityRole::Button)
                .a11y_children(AccessibilityChildren::ExcludeDescendants),
            RemoveButton {
                accessibility_label: remove_accessibility_label,
                action: remove_action,
            },
        ))
        .spacing(INPUT_CHIP_ICON_LABEL_SPACE)
        .height(INPUT_CHIP_CONTAINER_HEIGHT)
        .padding_with(EdgeInsets::new(
            0.0,
            0.0,
            INPUT_CHIP_LEADING_SPACE,
            INPUT_CHIP_WITH_TRAILING_ICON_TRAILING_SPACE,
        ))
        .background(RoundedRectangle::new(INPUT_CHIP_CONTAINER_CLIP_RADIUS).fill(Surface))
        .border_with(
            Border::new(Outline, INPUT_CHIP_UNSELECTED_OUTLINE_WIDTH)
                .corner_radius(INPUT_CHIP_CONTAINER_SHAPE),
        )
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

/// Creates a Material Design 3 input chip with the given semantic label.
#[must_use]
pub fn input_chip(label: impl IntoLabel) -> InputChip {
    InputChip::new(label)
}

fn selected_filter_chip_view(
    label: Label,
    accessibility_label: Str,
    selected: Binding<bool>,
    action: SharedAction,
) -> impl View {
    waterui::component::hstack((
        CheckmarkIcon::new(
            OnSecondaryContainer,
            FILTER_CHIP_ICON_SIZE,
            FILTER_CHIP_CHECKMARK_LINE_WIDTH,
        )
        .container_height(FILTER_CHIP_CONTAINER_HEIGHT),
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

struct RemoveButton {
    accessibility_label: Str,
    action: SharedAction,
}

impl View for RemoveButton {
    fn body(self, _env: &Environment) -> impl View {
        let accessibility_label = self.accessibility_label;
        let action = self.action;
        waterui::component::hstack((RemoveIcon,))
            .size(INPUT_CHIP_TRAILING_ICON_SIZE, INPUT_CHIP_TRAILING_ICON_SIZE)
            .on_tap(move |env: Environment| action.call(&env))
            .a11y_label(accessibility_label)
            .a11y_role(AccessibilityRole::Button)
            .a11y_children(AccessibilityChildren::ExcludeDescendants)
    }
}

struct RemoveIcon;

impl View for RemoveIcon {
    fn body(self, _env: &Environment) -> impl View {
        let line = || {
            Rectangle
                .fill(OnSurfaceVariant)
                .size(10.5, INPUT_CHIP_REMOVE_ICON_LINE_WIDTH)
        };
        waterui::component::zstack((line().rotation(45.0), line().rotation(-45.0)))
            .size(INPUT_CHIP_TRAILING_ICON_SIZE, INPUT_CHIP_TRAILING_ICON_SIZE)
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
        FILTER_CHIP_WITH_ICON_LEADING_SPACE, INPUT_CHIP_CONTAINER_HEIGHT,
        INPUT_CHIP_CONTAINER_SHAPE, INPUT_CHIP_ICON_LABEL_SPACE, INPUT_CHIP_LEADING_SPACE,
        INPUT_CHIP_TRAILING_ICON_SIZE, INPUT_CHIP_UNSELECTED_OUTLINE_WIDTH,
        INPUT_CHIP_WITH_TRAILING_ICON_TRAILING_SPACE,
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

    #[test]
    fn input_chip_tokens_match_material_web_v0_192() {
        assert_eq!(INPUT_CHIP_CONTAINER_HEIGHT, 32.0);
        assert_eq!(INPUT_CHIP_CONTAINER_SHAPE, 8.0);
        assert_eq!(INPUT_CHIP_UNSELECTED_OUTLINE_WIDTH, 1.0);
        assert_eq!(INPUT_CHIP_LEADING_SPACE, 16.0);
        assert_eq!(INPUT_CHIP_ICON_LABEL_SPACE, 8.0);
        assert_eq!(INPUT_CHIP_WITH_TRAILING_ICON_TRAILING_SPACE, 8.0);
        assert_eq!(INPUT_CHIP_TRAILING_ICON_SIZE, 18.0);
    }
}
