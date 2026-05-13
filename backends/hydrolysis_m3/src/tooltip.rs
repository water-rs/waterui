//! Material Design 3 tooltips composed from WaterUI primitives.

use core::fmt::{self, Debug};

use waterui::accessibility::AccessibilityRole;
use waterui::layout::padding::EdgeInsets;
use waterui::shape::{RoundedRectangle, ShapeExt as _};
use waterui::{Environment, Signal, Str, View, ViewExt as _};
use waterui_controls::label::{IntoLabel, Label};
use waterui_core::handler::{Handler, boxed_action};

use crate::color::{InverseOnSurface, InverseSurface, OnSurfaceVariant, Primary, SurfaceContainer};
use crate::theme::typography;

const PLAIN_TOOLTIP_CONTAINER_HEIGHT: f32 = 24.0;
const PLAIN_TOOLTIP_CONTAINER_SHAPE: f32 = 4.0;
const PLAIN_TOOLTIP_CONTAINER_CLIP_RADIUS: f32 =
    PLAIN_TOOLTIP_CONTAINER_SHAPE / PLAIN_TOOLTIP_CONTAINER_HEIGHT;
const PLAIN_TOOLTIP_TOP_SPACE: f32 = 4.0;
const PLAIN_TOOLTIP_BOTTOM_SPACE: f32 = 4.0;
const PLAIN_TOOLTIP_LEADING_SPACE: f32 = 8.0;
const PLAIN_TOOLTIP_TRAILING_SPACE: f32 = 8.0;
const RICH_TOOLTIP_CONTAINER_SHAPE: f32 = 12.0;
const RICH_TOOLTIP_CONTAINER_CLIP_RADIUS: f32 =
    RICH_TOOLTIP_CONTAINER_SHAPE / RICH_TOOLTIP_MAX_WIDTH;
const RICH_TOOLTIP_MAX_WIDTH: f32 = 312.0;
const RICH_TOOLTIP_PADDING: f32 = 16.0;
const RICH_TOOLTIP_CONTENT_SPACING: f32 = 4.0;
const RICH_TOOLTIP_ACTION_TOP_SPACE: f32 = 12.0;
const RICH_TOOLTIP_ACTION_HEIGHT: f32 = 40.0;

/// A Material Design 3 plain tooltip.
pub struct PlainTooltip {
    supporting_text: Label,
    accessibility_label: Str,
}

impl Debug for PlainTooltip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlainTooltip")
            .field("supporting_text", &self.supporting_text)
            .finish_non_exhaustive()
    }
}

impl PlainTooltip {
    /// Creates a plain tooltip with supporting text.
    #[must_use]
    pub fn new(supporting_text: impl IntoLabel) -> Self {
        let supporting_text = supporting_text.into_label();
        let accessibility_label = label_plain_text(&supporting_text);
        Self {
            supporting_text,
            accessibility_label,
        }
    }
}

impl View for PlainTooltip {
    fn body(self, _env: &Environment) -> impl View {
        self.supporting_text
            .font(typography::body_small())
            .foreground(InverseOnSurface)
            .padding_with(EdgeInsets::new(
                PLAIN_TOOLTIP_TOP_SPACE,
                PLAIN_TOOLTIP_BOTTOM_SPACE,
                PLAIN_TOOLTIP_LEADING_SPACE,
                PLAIN_TOOLTIP_TRAILING_SPACE,
            ))
            .background(
                RoundedRectangle::new(PLAIN_TOOLTIP_CONTAINER_CLIP_RADIUS).fill(InverseSurface),
            )
            .a11y_label(self.accessibility_label)
            .a11y_role(AccessibilityRole::Group)
    }
}

/// A Material Design 3 rich tooltip.
pub struct RichTooltip<Action = fn(&Environment)> {
    subhead: Label,
    supporting_text: Label,
    accessibility_label: Str,
    action: Option<(Label, Action)>,
}

impl<Action> Debug for RichTooltip<Action> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RichTooltip")
            .field("subhead", &self.subhead)
            .field("supporting_text", &self.supporting_text)
            .finish_non_exhaustive()
    }
}

impl RichTooltip<fn(&Environment)> {
    /// Creates a rich tooltip with a subhead and supporting text.
    #[must_use]
    pub fn new(subhead: impl IntoLabel, supporting_text: impl IntoLabel) -> Self {
        let subhead = subhead.into_label();
        let supporting_text = supporting_text.into_label();
        let accessibility_label = label_plain_text(&supporting_text);
        Self {
            subhead,
            supporting_text,
            accessibility_label,
            action: None,
        }
    }
}

impl<Action> RichTooltip<Action> {
    /// Adds a Material rich tooltip action.
    #[must_use]
    pub fn action<F, Args>(
        self,
        label: impl IntoLabel,
        action: F,
    ) -> RichTooltip<impl FnMut(&Environment)>
    where
        F: Handler<Args, ()> + 'static,
    {
        RichTooltip {
            subhead: self.subhead,
            supporting_text: self.supporting_text,
            accessibility_label: self.accessibility_label,
            action: Some((label.into_label(), boxed_action(action))),
        }
    }
}

impl<Action> View for RichTooltip<Action>
where
    Action: FnMut(&Environment) + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        let content = waterui::component::vstack((
            self.subhead
                .font(typography::title_small())
                .foreground(OnSurfaceVariant),
            self.supporting_text
                .font(typography::body_medium())
                .foreground(OnSurfaceVariant),
            rich_tooltip_action(self.action),
        ))
        .spacing(RICH_TOOLTIP_CONTENT_SPACING)
        .padding_with(RICH_TOOLTIP_PADDING)
        .background(
            RoundedRectangle::new(RICH_TOOLTIP_CONTAINER_CLIP_RADIUS).fill(SurfaceContainer),
        )
        .max_width(RICH_TOOLTIP_MAX_WIDTH);

        content
            .a11y_label(self.accessibility_label)
            .a11y_role(AccessibilityRole::Group)
    }
}

fn rich_tooltip_action<Action>(action: Option<(Label, Action)>) -> impl View
where
    Action: FnMut(&Environment) + 'static,
{
    let Some((label, mut action)) = action else {
        return waterui::component::hstack(((),)).anyview();
    };
    let accessibility_label = label_plain_text(&label);

    label
        .font(typography::label_large())
        .foreground(Primary)
        .height(RICH_TOOLTIP_ACTION_HEIGHT)
        .padding_with(EdgeInsets::new(
            RICH_TOOLTIP_ACTION_TOP_SPACE,
            0.0,
            0.0,
            0.0,
        ))
        .on_tap(move |env: Environment| action(&env))
        .a11y_label(accessibility_label)
        .a11y_role(AccessibilityRole::Button)
        .anyview()
}

fn label_plain_text(label: &Label) -> Str {
    label
        .semantic_text()
        .clone()
        .resolve(&Environment::new())
        .content
        .get()
        .to_plain()
        .into()
}

/// Creates a Material Design 3 plain tooltip.
#[must_use]
pub fn plain_tooltip(supporting_text: impl IntoLabel) -> PlainTooltip {
    PlainTooltip::new(supporting_text)
}

/// Creates a Material Design 3 rich tooltip.
#[must_use]
pub fn rich_tooltip(
    subhead: impl IntoLabel,
    supporting_text: impl IntoLabel,
) -> RichTooltip<fn(&Environment)> {
    RichTooltip::new(subhead, supporting_text)
}

#[cfg(test)]
mod tests {
    use super::{
        PLAIN_TOOLTIP_CONTAINER_HEIGHT, PLAIN_TOOLTIP_CONTAINER_SHAPE, PLAIN_TOOLTIP_LEADING_SPACE,
        PLAIN_TOOLTIP_TOP_SPACE, RICH_TOOLTIP_CONTAINER_SHAPE, RICH_TOOLTIP_MAX_WIDTH,
        RICH_TOOLTIP_PADDING,
    };

    #[test]
    fn plain_tooltip_tokens_match_material_web_v0_192() {
        assert_eq!(PLAIN_TOOLTIP_CONTAINER_HEIGHT, 24.0);
        assert_eq!(PLAIN_TOOLTIP_CONTAINER_SHAPE, 4.0);
        assert_eq!(PLAIN_TOOLTIP_TOP_SPACE, 4.0);
        assert_eq!(PLAIN_TOOLTIP_LEADING_SPACE, 8.0);
    }

    #[test]
    fn rich_tooltip_tokens_match_material_web_v0_192() {
        assert_eq!(RICH_TOOLTIP_CONTAINER_SHAPE, 12.0);
        assert_eq!(RICH_TOOLTIP_MAX_WIDTH, 312.0);
        assert_eq!(RICH_TOOLTIP_PADDING, 16.0);
    }
}
