//! Material Design 3 split button composed from `WaterUI` primitives.
//!
//! A split button is two buttons sharing one silhouette: a leading button that
//! performs the action and a trailing one that opens a menu of alternatives.
//! Their touching corners are tucked in while their outer corners stay fully
//! round, which is what makes the pair read as a single control with a seam
//! rather than as two adjacent buttons.

use core::fmt::{self, Debug};
use core::marker::PhantomData;

use waterui::accessibility::{AccessibilityChildren, AccessibilityRole};
use waterui::color::Color;
use waterui::component::hstack;
use waterui::layout::padding::EdgeInsets;
use waterui::shape::{FilledShape, Path, ShapeExt as _, UnevenRoundedRectangle};
use waterui::{Environment, Str, View, ViewExt as _};
use waterui_controls::label::{IntoLabel, Label};
use waterui_core::handler::{Handler, boxed_action};

use crate::color::{OnPrimary, OnSecondaryContainer, Primary, SecondaryContainer};
use crate::semantics::interaction_style;

/// `SplitButtonSmallTokens.ContainerHeight`.
const CONTAINER_HEIGHT: f32 = 40.0;
/// `SplitButtonSmallTokens.BetweenSpace`: the seam between the two halves.
const BETWEEN_SPACE: f32 = 2.0;
/// `SplitButtonSmallTokens.LeadingButtonLeadingSpace`.
const LEADING_BUTTON_LEADING_SPACE: f32 = 16.0;
/// `SplitButtonSmallTokens.LeadingButtonTrailingSpace`.
const LEADING_BUTTON_TRAILING_SPACE: f32 = 12.0;
/// `SplitButtonSmallTokens.TrailingButtonLeadingSpace` / `TrailingSpace`.
const TRAILING_BUTTON_SPACE: f32 = 13.0;
/// `SplitButtonSmallTokens.TrailingIconSize`.
const TRAILING_ICON_SIZE: f32 = 22.0;
/// `SplitButtonSmallTokens.InnerCornerCornerSize`, `CornerValueExtraSmall`:
/// the radius on the two corners either side of the seam.
const INNER_CORNER_RADIUS: f32 = 4.0;

/// The outer corners are `CornerFull`, which on this container is half its
/// height.
const OUTER_CORNER_RADIUS: f32 = CONTAINER_HEIGHT / 2.0;

/// Corner radii normalize against the shorter side of the shape, and both
/// halves are exactly `CONTAINER_HEIGHT` tall.
const fn normalized(radius: f32) -> f32 {
    radius / CONTAINER_HEIGHT
}

/// Colour token set for a split button variant.
pub trait SplitButtonVariantTokens: Default + 'static {
    /// Container colour of both halves.
    fn container_color() -> Color;

    /// Label and icon colour.
    fn content_color() -> Color;
}

/// Filled split button tokens.
#[derive(Debug, Clone, Copy, Default)]
pub struct FilledSplitButton;

/// Tonal split button tokens.
#[derive(Debug, Clone, Copy, Default)]
pub struct TonalSplitButton;

impl SplitButtonVariantTokens for FilledSplitButton {
    fn container_color() -> Color {
        Primary.into()
    }

    fn content_color() -> Color {
        OnPrimary.into()
    }
}

impl SplitButtonVariantTokens for TonalSplitButton {
    fn container_color() -> Color {
        SecondaryContainer.into()
    }

    fn content_color() -> Color {
        OnSecondaryContainer.into()
    }
}

/// A Material Design 3 split button.
pub struct SplitButton<
    Action = fn(&Environment),
    TrailingAction = fn(&Environment),
    Tokens = FilledSplitButton,
> {
    label: Label,
    trailing_accessibility_label: Str,
    action: Action,
    trailing_action: TrailingAction,
    tokens: PhantomData<Tokens>,
}

impl<Action, TrailingAction, Tokens> Debug for SplitButton<Action, TrailingAction, Tokens> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SplitButton")
            .field("label", &self.label)
            .field(
                "trailing_accessibility_label",
                &self.trailing_accessibility_label,
            )
            .finish_non_exhaustive()
    }
}

impl SplitButton {
    /// Creates a split button.
    ///
    /// `trailing_accessibility_label` names what the trailing half opens; it
    /// has no visible text of its own, only a chevron, so assistive technology
    /// has nothing else to announce.
    #[must_use]
    pub fn new(label: impl IntoLabel, trailing_accessibility_label: impl Into<Str>) -> Self {
        Self {
            label: label.into_label(),
            trailing_accessibility_label: trailing_accessibility_label.into(),
            action: noop,
            trailing_action: noop,
            tokens: PhantomData,
        }
    }
}

impl<Action, TrailingAction, Tokens> SplitButton<Action, TrailingAction, Tokens> {
    /// Uses tonal split button tokens.
    #[must_use]
    pub fn tonal(self) -> SplitButton<Action, TrailingAction, TonalSplitButton> {
        SplitButton {
            label: self.label,
            trailing_accessibility_label: self.trailing_accessibility_label,
            action: self.action,
            trailing_action: self.trailing_action,
            tokens: PhantomData,
        }
    }

    /// Sets the action the leading half performs.
    #[must_use]
    pub fn action<F, Args>(
        self,
        action: F,
    ) -> SplitButton<impl FnMut(&Environment), TrailingAction, Tokens>
    where
        F: Handler<Args, ()> + 'static,
    {
        SplitButton {
            label: self.label,
            trailing_accessibility_label: self.trailing_accessibility_label,
            action: boxed_action(action),
            trailing_action: self.trailing_action,
            tokens: PhantomData,
        }
    }

    /// Sets the action the trailing half performs.
    #[must_use]
    pub fn trailing_action<F, Args>(
        self,
        action: F,
    ) -> SplitButton<Action, impl FnMut(&Environment), Tokens>
    where
        F: Handler<Args, ()> + 'static,
    {
        SplitButton {
            label: self.label,
            trailing_accessibility_label: self.trailing_accessibility_label,
            action: self.action,
            trailing_action: boxed_action(action),
            tokens: PhantomData,
        }
    }
}

impl<Action, TrailingAction, Tokens> View for SplitButton<Action, TrailingAction, Tokens>
where
    Action: FnMut(&Environment) + 'static,
    TrailingAction: FnMut(&Environment) + 'static,
    Tokens: SplitButtonVariantTokens,
{
    fn body(self, _env: &Environment) -> impl View {
        let mut action = self.action;
        let mut trailing_action = self.trailing_action;
        let outer = normalized(OUTER_CORNER_RADIUS);
        let inner = normalized(INNER_CORNER_RADIUS);

        // Round on the outside, tucked in against the seam.
        let leading_shape = UnevenRoundedRectangle::new(outer, inner, outer, inner);
        let trailing_shape = UnevenRoundedRectangle::new(inner, outer, inner, outer);

        let leading = self
            .label
            .foreground(Tokens::content_color())
            .padding_with(EdgeInsets::new(
                0.0,
                0.0,
                LEADING_BUTTON_LEADING_SPACE,
                LEADING_BUTTON_TRAILING_SPACE,
            ))
            .height(CONTAINER_HEIGHT)
            .background(leading_shape.fill(Tokens::container_color()))
            .on_tap(move |env: Environment| action(&env))
            .a11y_role(AccessibilityRole::Button)
            .install(interaction_style(
                Tokens::content_color(),
                f64::from(OUTER_CORNER_RADIUS),
            ));

        let chevron = ChevronIcon {
            color: Tokens::content_color(),
            size: TRAILING_ICON_SIZE,
        };
        let trailing = chevron
            .padding_with(EdgeInsets::new(
                0.0,
                0.0,
                TRAILING_BUTTON_SPACE,
                TRAILING_BUTTON_SPACE,
            ))
            .height(CONTAINER_HEIGHT)
            .background(trailing_shape.fill(Tokens::container_color()))
            .on_tap(move |env: Environment| trailing_action(&env))
            .a11y_label(self.trailing_accessibility_label)
            .a11y_role(AccessibilityRole::Button)
            .a11y_children(AccessibilityChildren::ExcludeDescendants)
            .install(interaction_style(
                Tokens::content_color(),
                f64::from(OUTER_CORNER_RADIUS),
            ));

        hstack((leading, trailing)).spacing(BETWEEN_SPACE)
    }
}

/// The trailing half's chevron, sized and tinted by the variant.
#[derive(Debug, Clone)]
struct ChevronIcon {
    color: Color,
    size: f32,
}

impl View for ChevronIcon {
    fn body(self, _env: &Environment) -> impl View {
        // The outline is authored on Material's 24dp grid; `FilledShape` wants
        // unit-square commands, so it is normalized here rather than restated.
        let grid = crate::icon_paths::icon_grid_size();
        let normalize = |value: f64| {
            #[allow(
                clippy::cast_possible_truncation,
                reason = "icon grid coordinates are small and exact in f32"
            )]
            let normalized = (value / grid) as f32;
            normalized
        };
        let mut corners = crate::icon_paths::CHEVRON_DOWN_OUTLINE.into_iter();
        let (start_x, start_y) = corners.next().expect("chevron outline is non-empty");
        let path = corners
            .fold(
                Path::new().move_to(normalize(start_x), normalize(start_y)),
                |path, (x, y)| path.line_to(normalize(x), normalize(y)),
            )
            .close();
        FilledShape::new(path, self.color).size(self.size, self.size)
    }
}

const fn noop(_env: &Environment) {}

/// Creates a Material Design 3 split button.
#[must_use]
pub fn split_button(
    label: impl IntoLabel,
    trailing_accessibility_label: impl Into<Str>,
) -> SplitButton {
    SplitButton::new(label, trailing_accessibility_label)
}

#[cfg(test)]
mod tests {
    use super::{
        BETWEEN_SPACE, CONTAINER_HEIGHT, INNER_CORNER_RADIUS, LEADING_BUTTON_LEADING_SPACE,
        LEADING_BUTTON_TRAILING_SPACE, OUTER_CORNER_RADIUS, TRAILING_BUTTON_SPACE,
        TRAILING_ICON_SIZE, normalized,
    };

    /// Values from `androidx.compose.material3.tokens.SplitButtonSmallTokens`.
    #[test]
    fn split_button_tokens_match_compose_split_button_tokens() {
        assert_eq!(CONTAINER_HEIGHT, 40.0);
        assert_eq!(BETWEEN_SPACE, 2.0);
        assert_eq!(LEADING_BUTTON_LEADING_SPACE, 16.0);
        assert_eq!(LEADING_BUTTON_TRAILING_SPACE, 12.0);
        assert_eq!(TRAILING_BUTTON_SPACE, 13.0);
        assert_eq!(TRAILING_ICON_SIZE, 22.0);
        // InnerCornerCornerSize is ShapeTokens.CornerValueExtraSmall.
        assert_eq!(INNER_CORNER_RADIUS, 4.0);
        // The outer corners are CornerFull on a 40dp container.
        assert_eq!(OUTER_CORNER_RADIUS, 20.0);
    }

    /// Corner radii are normalized against the shorter side, and a normalized
    /// radius above 0.5 would be clamped — which would silently round the
    /// tucked-in corners as much as the outer ones.
    #[test]
    fn corner_radii_stay_within_the_normalized_range() {
        let outer = normalized(OUTER_CORNER_RADIUS);
        let inner = normalized(INNER_CORNER_RADIUS);
        assert!((outer - 0.5).abs() < 1e-6, "outer corner is fully round");
        assert!(inner > 0.0 && inner < outer, "the seam is tucked in");
    }
}
