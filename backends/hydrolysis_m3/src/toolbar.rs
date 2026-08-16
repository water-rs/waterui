//! Material Design 3 toolbars composed from `WaterUI` primitives.
//!
//! Both hold a row of actions and differ in where they sit. A floating toolbar
//! rides above the content as a rounded bar inset from the edges; a docked one
//! is pinned flush against an edge and spans it. That difference is the whole
//! component, which is why they share their contents and split on shape,
//! spacing and how far they inset.

use core::fmt::{self, Debug};

// `WaterUI` has no `Toolbar` role, so a toolbar announces as the group of
// controls it is.
use waterui::accessibility::AccessibilityRole;
use waterui::layout::padding::EdgeInsets;
use waterui::shape::{Capsule, ShapeExt as _};
use waterui::{Environment, View, ViewExt as _};
use waterui_core::view::TupleViews;

use crate::color::{OnPrimaryContainer, OnSurface, PrimaryContainer, SurfaceContainer};
use crate::elevation::{MaterialElevationLevel, material_elevation};

/// `FloatingToolbarTokens.ContainerHeight` and `DockedToolbarTokens
/// .ContainerHeight`, which agree.
const CONTAINER_HEIGHT: f32 = 64.0;
/// `FloatingToolbarTokens.ContainerLeadingSpace` / `ContainerTrailingSpace`.
const FLOATING_HORIZONTAL_SPACE: f32 = 8.0;
/// `FloatingToolbarTokens.ContainerBetweenSpace`.
const FLOATING_BETWEEN_SPACE: f32 = 4.0;
/// `FloatingToolbarTokens.ContainerExternalPadding`: how far the bar sits from
/// the edges it floats above.
const FLOATING_EXTERNAL_PADDING: f32 = 16.0;
/// `DockedToolbarTokens.ContainerLeadingSpace` / `ContainerTrailingSpace`.
const DOCKED_HORIZONTAL_SPACE: f32 = 16.0;
/// `DockedToolbarTokens.ContainerMinSpacing`.
const DOCKED_MIN_SPACING: f32 = 4.0;
/// `DockedToolbarTokens.ContainerMaxSpacing`.
const DOCKED_MAX_SPACING: f32 = 32.0;

/// How a floating toolbar is coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FloatingToolbarStyle {
    /// `StandardContainerColor`: the toolbar recedes into the surface.
    #[default]
    Standard,
    /// `VibrantContainerColor`: the toolbar asserts itself in the primary tone.
    Vibrant,
}

impl FloatingToolbarStyle {
    /// The container colour for this style.
    #[must_use]
    fn container_color(self) -> waterui::color::Color {
        match self {
            Self::Standard => SurfaceContainer.into(),
            Self::Vibrant => PrimaryContainer.into(),
        }
    }

    /// The colour actions take on this container.
    #[must_use]
    fn content_color(self) -> waterui::color::Color {
        match self {
            // `VibrantButtonUnselectedIconColor` / `TextColor`; the standard
            // toolbar sits on a surface, so its content is on-surface.
            Self::Standard => OnSurface.into(),
            Self::Vibrant => OnPrimaryContainer.into(),
        }
    }
}

/// A Material Design 3 floating toolbar.
pub struct FloatingToolbar<Actions> {
    actions: Actions,
    style: FloatingToolbarStyle,
}

impl<Actions> Debug for FloatingToolbar<Actions> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FloatingToolbar")
            .field("style", &self.style)
            .finish_non_exhaustive()
    }
}

impl<Actions> FloatingToolbar<Actions> {
    /// Creates a floating toolbar holding `actions`.
    #[must_use]
    pub const fn new(actions: Actions) -> Self {
        Self {
            actions,
            style: FloatingToolbarStyle::Standard,
        }
    }

    /// Uses the vibrant colour set.
    #[must_use]
    pub const fn vibrant(mut self) -> Self {
        self.style = FloatingToolbarStyle::Vibrant;
        self
    }
}

impl<Actions> View for FloatingToolbar<Actions>
where
    Actions: TupleViews + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        let style = self.style;
        material_elevation(
            MaterialElevationLevel::LEVEL3,
            waterui::component::hstack(self.actions)
                .spacing(FLOATING_BETWEEN_SPACE)
                .padding_with(EdgeInsets::new(
                    0.0,
                    0.0,
                    FLOATING_HORIZONTAL_SPACE,
                    FLOATING_HORIZONTAL_SPACE,
                ))
                .height(CONTAINER_HEIGHT)
                .foreground(style.content_color())
                // `FloatingToolbarTokens.ContainerShape` is `CornerFull`, which a
                // capsule is by construction at any height.
                .background(Capsule.fill(style.container_color())),
        )
        .padding_with(FLOATING_EXTERNAL_PADDING)
        .a11y_role(AccessibilityRole::Group)
    }
}

/// A Material Design 3 docked toolbar.
pub struct DockedToolbar<Actions> {
    actions: Actions,
    spacing: f32,
}

impl<Actions> Debug for DockedToolbar<Actions> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DockedToolbar")
            .field("spacing", &self.spacing)
            .finish_non_exhaustive()
    }
}

impl<Actions> DockedToolbar<Actions> {
    /// Creates a docked toolbar holding `actions`.
    #[must_use]
    pub const fn new(actions: Actions) -> Self {
        Self {
            actions,
            spacing: DOCKED_MIN_SPACING,
        }
    }

    /// Sets the gap between actions.
    ///
    /// A docked toolbar spans its edge, so how far apart its actions sit is the
    /// caller's call. `DockedToolbarTokens` bounds it, and values outside that
    /// range are clamped rather than honoured — beyond it the row stops reading
    /// as one toolbar.
    #[must_use]
    pub const fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing.clamp(DOCKED_MIN_SPACING, DOCKED_MAX_SPACING);
        self
    }
}

impl<Actions> View for DockedToolbar<Actions>
where
    Actions: TupleViews + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        // `DockedToolbarTokens.ContainerShape` is `CornerNone`: it is flush
        // against its edge, so it takes no corner and no external padding.
        waterui::component::hstack(self.actions)
            .spacing(self.spacing)
            .padding_with(EdgeInsets::new(
                0.0,
                0.0,
                DOCKED_HORIZONTAL_SPACE,
                DOCKED_HORIZONTAL_SPACE,
            ))
            .height(CONTAINER_HEIGHT)
            .max_width(f32::INFINITY)
            .foreground(OnSurface)
            .background(SurfaceContainer)
            .a11y_role(AccessibilityRole::Group)
    }
}

/// Creates a Material Design 3 floating toolbar.
#[must_use]
pub const fn floating_toolbar<Actions>(actions: Actions) -> FloatingToolbar<Actions> {
    FloatingToolbar::new(actions)
}

/// Creates a Material Design 3 docked toolbar.
#[must_use]
pub const fn docked_toolbar<Actions>(actions: Actions) -> DockedToolbar<Actions> {
    DockedToolbar::new(actions)
}

#[cfg(test)]
mod tests {
    use super::{
        CONTAINER_HEIGHT, DOCKED_HORIZONTAL_SPACE, DOCKED_MAX_SPACING, DOCKED_MIN_SPACING,
        DockedToolbar, FLOATING_BETWEEN_SPACE, FLOATING_EXTERNAL_PADDING,
        FLOATING_HORIZONTAL_SPACE,
    };

    /// Values from `androidx.compose.material3.tokens.FloatingToolbarTokens`
    /// and `DockedToolbarTokens`.
    #[test]
    fn toolbar_tokens_match_compose_toolbar_tokens() {
        // Both toolbars stand the same height.
        assert_eq!(CONTAINER_HEIGHT, 64.0);
        assert_eq!(FLOATING_HORIZONTAL_SPACE, 8.0);
        assert_eq!(FLOATING_BETWEEN_SPACE, 4.0);
        assert_eq!(FLOATING_EXTERNAL_PADDING, 16.0);
        assert_eq!(DOCKED_HORIZONTAL_SPACE, 16.0);
        assert_eq!(DOCKED_MIN_SPACING, 4.0);
        assert_eq!(DOCKED_MAX_SPACING, 32.0);
    }

    /// A docked toolbar's spacing is bounded by its tokens, so a caller cannot
    /// spread its actions until the row stops reading as one toolbar.
    #[test]
    fn docked_toolbar_spacing_is_clamped_to_its_token_range() {
        assert_eq!(
            DockedToolbar::new(()).spacing(0.0).spacing,
            DOCKED_MIN_SPACING
        );
        assert_eq!(
            DockedToolbar::new(()).spacing(1_000.0).spacing,
            DOCKED_MAX_SPACING
        );
        // A value inside the range is honoured exactly.
        assert_eq!(DockedToolbar::new(()).spacing(12.0).spacing, 12.0);
        // The default is the tokens' minimum.
        assert_eq!(DockedToolbar::new(()).spacing, DOCKED_MIN_SPACING);
    }
}
