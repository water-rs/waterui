//! Material Design 3 segmented buttons composed from `WaterUI` primitives.

use core::fmt::{self, Debug};

use waterui::accessibility::{AccessibilityChildren, AccessibilityRole, AccessibilityState};
use waterui::border::Border;
use waterui::color::Color;
use waterui::component::hstack;
use waterui::layout::padding::EdgeInsets;
use waterui::reactive::SignalExt as _;
use waterui::shape::{Rectangle, ShapeExt as _, UnevenRoundedRectangle};
use waterui::widget::condition::when;
use waterui::{AnyView, Binding, Environment, Str, View, ViewExt as _};
use waterui_controls::label::{IntoLabel, Label};
use waterui_core::handler::{Handler, SharedAction, boxed_action};
use waterui_core::view::TupleViews;

use crate::color::{OnSecondaryContainer, OnSurface, Outline, SecondaryContainer, Surface};
use crate::icons::CheckmarkIcon;
use crate::semantics::label_plain_text;
use crate::theme::typography;

const OUTLINED_SEGMENTED_BUTTON_CONTAINER_HEIGHT: f32 = 40.0;
const OUTLINED_SEGMENTED_BUTTON_CONTAINER_SHAPE: f32 = 20.0;
const OUTLINED_SEGMENTED_BUTTON_CONTAINER_CLIP_RADIUS: f32 =
    OUTLINED_SEGMENTED_BUTTON_CONTAINER_SHAPE / OUTLINED_SEGMENTED_BUTTON_CONTAINER_HEIGHT;
const OUTLINED_SEGMENTED_BUTTON_OUTLINE_WIDTH: f32 = 1.0;
const OUTLINED_SEGMENTED_BUTTON_LEADING_SPACE: f32 = 12.0;
const OUTLINED_SEGMENTED_BUTTON_TRAILING_SPACE: f32 = 12.0;
const OUTLINED_SEGMENTED_BUTTON_ICON_SIZE: f32 = 18.0;
const OUTLINED_SEGMENTED_BUTTON_CHECKMARK_LINE_WIDTH: f32 = 2.0;
const OUTLINED_SEGMENTED_BUTTON_ICON_LABEL_SPACE: f32 = 8.0;

/// Logical corner treatment for a segmented button inside a set.
#[derive(Debug, Clone, Copy)]
pub struct SegmentedButtonShape {
    top_leading: f32,
    top_trailing: f32,
    bottom_leading: f32,
    bottom_trailing: f32,
}

impl SegmentedButtonShape {
    /// Shape for a single standalone segmented button.
    #[must_use]
    pub const fn single() -> Self {
        Self::all(OUTLINED_SEGMENTED_BUTTON_CONTAINER_CLIP_RADIUS)
    }

    /// Shape for the leading segment in a horizontal set.
    #[must_use]
    pub const fn start() -> Self {
        Self::new(
            OUTLINED_SEGMENTED_BUTTON_CONTAINER_CLIP_RADIUS,
            0.0,
            OUTLINED_SEGMENTED_BUTTON_CONTAINER_CLIP_RADIUS,
            0.0,
        )
    }

    /// Shape for a middle segment in a horizontal set.
    #[must_use]
    pub const fn middle() -> Self {
        Self::all(0.0)
    }

    /// Shape for the trailing segment in a horizontal set.
    #[must_use]
    pub const fn end() -> Self {
        Self::new(
            0.0,
            OUTLINED_SEGMENTED_BUTTON_CONTAINER_CLIP_RADIUS,
            0.0,
            OUTLINED_SEGMENTED_BUTTON_CONTAINER_CLIP_RADIUS,
        )
    }

    const fn all(radius: f32) -> Self {
        Self::new(radius, radius, radius, radius)
    }

    const fn new(
        top_leading: f32,
        top_trailing: f32,
        bottom_leading: f32,
        bottom_trailing: f32,
    ) -> Self {
        Self {
            top_leading,
            top_trailing,
            bottom_leading,
            bottom_trailing,
        }
    }

    const fn shape(self) -> UnevenRoundedRectangle {
        UnevenRoundedRectangle::new(
            self.top_leading,
            self.top_trailing,
            self.bottom_leading,
            self.bottom_trailing,
        )
    }

    const fn has_leading_separator(self) -> bool {
        self.top_leading == 0.0 && self.bottom_leading == 0.0
    }
}

/// A Material Design 3 outlined segmented button.
pub struct OutlinedSegmentedButton<Action = fn(&Environment)> {
    label: Label,
    accessibility_label: Str,
    selected: Binding<bool>,
    shape: SegmentedButtonShape,
    action: Action,
}

impl<Action> Debug for OutlinedSegmentedButton<Action> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutlinedSegmentedButton")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl OutlinedSegmentedButton<fn(&Environment)> {
    /// Creates an outlined segmented button bound to the provided selected state.
    #[must_use]
    pub fn new(label: impl IntoLabel, selected: &Binding<bool>) -> Self {
        let label = label.into_label();
        let accessibility_label = label_plain_text(&label);
        Self {
            label,
            accessibility_label,
            selected: selected.clone(),
            shape: SegmentedButtonShape::single(),
            action: noop,
        }
    }
}

impl<Action> OutlinedSegmentedButton<Action> {
    /// Uses the leading-segment shape.
    #[must_use]
    pub const fn start(mut self) -> Self {
        self.shape = SegmentedButtonShape::start();
        self
    }

    /// Uses the middle-segment shape.
    #[must_use]
    pub const fn middle(mut self) -> Self {
        self.shape = SegmentedButtonShape::middle();
        self
    }

    /// Uses the trailing-segment shape.
    #[must_use]
    pub const fn end(mut self) -> Self {
        self.shape = SegmentedButtonShape::end();
        self
    }

    /// Sets the action performed after this button toggles its selected state.
    #[must_use]
    pub fn action<F, Args>(self, action: F) -> OutlinedSegmentedButton<impl FnMut(&Environment)>
    where
        F: Handler<Args, ()> + 'static,
    {
        OutlinedSegmentedButton {
            label: self.label,
            accessibility_label: self.accessibility_label,
            selected: self.selected,
            shape: self.shape,
            action: boxed_action(action),
        }
    }
}

impl<Action> View for OutlinedSegmentedButton<Action>
where
    Action: FnMut(&Environment) + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        let mut action = self.action;
        let action = SharedAction::new(move |env: Environment| action(&env));
        let selected_for_state = self.selected.clone();
        let selected_for_tap = self.selected.clone();
        let accessibility_state = self
            .selected
            .map(|selected| AccessibilityState::new().selected(selected));
        let selected_label = self.label.clone();
        let unselected_label = self.label;
        let selected_shape = self.shape;
        let unselected_shape = self.shape;
        let selected_accessibility_label = self.accessibility_label.clone();
        let unselected_accessibility_label = self.accessibility_label;
        let selected_action = action.clone();
        let unselected_action = action;
        let selected_tap_state = selected_for_tap.clone();
        let unselected_tap_state = selected_for_tap;

        when(selected_for_state, move || {
            segmented_button_view(
                selected_label.clone(),
                Some(
                    CheckmarkIcon::new(
                        OnSecondaryContainer,
                        OUTLINED_SEGMENTED_BUTTON_ICON_SIZE,
                        OUTLINED_SEGMENTED_BUTTON_CHECKMARK_LINE_WIDTH,
                    )
                    .container_height(OUTLINED_SEGMENTED_BUTTON_CONTAINER_HEIGHT),
                ),
                selected_shape,
                true,
                selected_accessibility_label.clone(),
                selected_tap_state.clone(),
                selected_action.clone(),
            )
        })
        .otherwise(move || {
            segmented_button_view(
                unselected_label.clone(),
                None::<CheckmarkIcon<OnSecondaryContainer>>,
                unselected_shape,
                false,
                unselected_accessibility_label.clone(),
                unselected_tap_state.clone(),
                unselected_action.clone(),
            )
        })
        .a11y_state_signal(accessibility_state)
    }
}

/// A Material Design 3 outlined segmented button set.
pub struct OutlinedSegmentedButtonSet<Content> {
    content: Content,
    accessibility_label: Str,
}

impl<Content> Debug for OutlinedSegmentedButtonSet<Content> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutlinedSegmentedButtonSet")
            .field("accessibility_label", &self.accessibility_label)
            .finish_non_exhaustive()
    }
}

impl<Content> OutlinedSegmentedButtonSet<Content> {
    /// Creates an outlined segmented button set with two or more button children.
    #[must_use]
    pub fn new(content: Content) -> Self {
        Self {
            content,
            accessibility_label: "Segmented buttons".into(),
        }
    }

    /// Sets the set accessibility label.
    #[must_use]
    pub fn label(mut self, label: impl Into<Str>) -> Self {
        self.accessibility_label = label.into();
        self
    }
}

impl<Content> View for OutlinedSegmentedButtonSet<Content>
where
    Content: TupleViews + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        hstack(self.content)
            .spacing(0.0)
            .border_with(
                Border::new(Outline, OUTLINED_SEGMENTED_BUTTON_OUTLINE_WIDTH)
                    .corner_radius(OUTLINED_SEGMENTED_BUTTON_CONTAINER_SHAPE),
            )
            .a11y_label(self.accessibility_label)
            .a11y_role(AccessibilityRole::Group)
    }
}

/// Creates a Material Design 3 outlined segmented button.
#[must_use]
pub fn outlined_segmented_button(
    label: impl IntoLabel,
    selected: &Binding<bool>,
) -> OutlinedSegmentedButton {
    OutlinedSegmentedButton::new(label, selected)
}

/// Creates a Material Design 3 outlined segmented button set.
#[must_use]
pub fn outlined_segmented_button_set<Content>(
    content: Content,
) -> OutlinedSegmentedButtonSet<Content> {
    OutlinedSegmentedButtonSet::new(content)
}

fn segmented_button_view<Leading>(
    label: Label,
    leading: Option<Leading>,
    shape: SegmentedButtonShape,
    selected: bool,
    accessibility_label: Str,
    selected_state: Binding<bool>,
    action: SharedAction,
) -> impl View
where
    Leading: View + 'static,
{
    let foreground: Color = if selected {
        OnSecondaryContainer.into()
    } else {
        OnSurface.into()
    };
    let background: Color = if selected {
        SecondaryContainer.into()
    } else {
        Surface.with_opacity(0.0).into()
    };
    let label = label.font(typography::label_large()).foreground(foreground);
    let inner = match leading {
        Some(leading) => AnyView::new(
            hstack((leading, label))
                .spacing(OUTLINED_SEGMENTED_BUTTON_ICON_LABEL_SPACE)
                .padding_with(EdgeInsets::new(
                    0.0,
                    0.0,
                    OUTLINED_SEGMENTED_BUTTON_LEADING_SPACE,
                    OUTLINED_SEGMENTED_BUTTON_TRAILING_SPACE,
                )),
        ),
        None => AnyView::new(hstack((label,)).padding_with(EdgeInsets::new(
            0.0,
            0.0,
            OUTLINED_SEGMENTED_BUTTON_LEADING_SPACE,
            OUTLINED_SEGMENTED_BUTTON_TRAILING_SPACE,
        ))),
    };
    let content = if shape.has_leading_separator() {
        AnyView::new(
            hstack((
                Rectangle
                    .fill(Outline)
                    .width(OUTLINED_SEGMENTED_BUTTON_OUTLINE_WIDTH)
                    .height(OUTLINED_SEGMENTED_BUTTON_CONTAINER_HEIGHT),
                inner,
            ))
            .spacing(0.0),
        )
    } else {
        inner
    };

    content
        .height(OUTLINED_SEGMENTED_BUTTON_CONTAINER_HEIGHT)
        .background(shape.shape().fill(background))
        .on_tap(move |env: Environment| {
            selected_state.set(!selected_state.get());
            action.call(&env);
        })
        .a11y_label(accessibility_label)
        .a11y_role(AccessibilityRole::Button)
        .a11y_children(AccessibilityChildren::ExcludeDescendants)
}

const fn noop(_env: &Environment) {}

#[cfg(test)]
mod tests {
    use super::{
        OUTLINED_SEGMENTED_BUTTON_CONTAINER_HEIGHT, OUTLINED_SEGMENTED_BUTTON_CONTAINER_SHAPE,
        OUTLINED_SEGMENTED_BUTTON_ICON_LABEL_SPACE, OUTLINED_SEGMENTED_BUTTON_ICON_SIZE,
        OUTLINED_SEGMENTED_BUTTON_LEADING_SPACE, OUTLINED_SEGMENTED_BUTTON_OUTLINE_WIDTH,
        OUTLINED_SEGMENTED_BUTTON_TRAILING_SPACE,
    };

    #[test]
    fn outlined_segmented_button_tokens_match_material_web_v0_192() {
        assert_eq!(OUTLINED_SEGMENTED_BUTTON_CONTAINER_HEIGHT, 40.0);
        assert_eq!(OUTLINED_SEGMENTED_BUTTON_CONTAINER_SHAPE, 20.0);
        assert_eq!(OUTLINED_SEGMENTED_BUTTON_OUTLINE_WIDTH, 1.0);
        assert_eq!(OUTLINED_SEGMENTED_BUTTON_LEADING_SPACE, 12.0);
        assert_eq!(OUTLINED_SEGMENTED_BUTTON_TRAILING_SPACE, 12.0);
        assert_eq!(OUTLINED_SEGMENTED_BUTTON_ICON_SIZE, 18.0);
        assert_eq!(OUTLINED_SEGMENTED_BUTTON_ICON_LABEL_SPACE, 8.0);
    }
}
