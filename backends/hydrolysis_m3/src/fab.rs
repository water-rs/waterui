//! Material Design 3 floating action buttons composed from `WaterUI` primitives.

use core::fmt::{self, Debug};
use core::marker::PhantomData;

use waterui::accessibility::{AccessibilityChildren, AccessibilityRole};
use waterui::color::Color;
use waterui::component::zstack;
use waterui::layout::padding::EdgeInsets;
use waterui::shape::{RoundedRectangle, ShapeExt as _};
use waterui::{Environment, Signal, Str, View, ViewExt as _};
use waterui_controls::label::{IntoLabel, Label};
use waterui_core::handler::{Handler, boxed_action};

use crate::color::{
    OnPrimaryContainer, OnSecondaryContainer, OnTertiaryContainer, Primary, PrimaryContainer,
    SecondaryContainer, Shadow as ShadowColor, SurfaceContainerHigh, TertiaryContainer,
};
use crate::semantics::interaction_style;
use crate::theme::typography;

const FAB_CONTAINER_HEIGHT: f32 = 56.0;
const FAB_CONTAINER_WIDTH: f32 = 56.0;
const FAB_CONTAINER_SHAPE: f32 = 16.0;
const FAB_CONTAINER_CLIP_RADIUS: f32 = FAB_CONTAINER_SHAPE / FAB_CONTAINER_HEIGHT;
const FAB_ICON_SIZE: f32 = 24.0;
const EXTENDED_FAB_HEIGHT: f32 = 56.0;
const EXTENDED_FAB_SHAPE: f32 = 16.0;
const EXTENDED_FAB_CLIP_RADIUS: f32 = EXTENDED_FAB_SHAPE / EXTENDED_FAB_HEIGHT;
const EXTENDED_FAB_LEADING_SPACE_WITHOUT_ICON: f32 = 20.0;
const EXTENDED_FAB_TRAILING_SPACE: f32 = 20.0;
const FAB_ELEVATION_AMBIENT_OFFSET_Y: f32 = 2.0;
const FAB_ELEVATION_AMBIENT_OPACITY: f32 = 0.08;
const FAB_ELEVATION_KEY_OFFSET_Y: f32 = 1.0;
const FAB_ELEVATION_KEY_OPACITY: f32 = 0.12;

/// Color token set for a FAB variant.
pub trait FabVariantTokens: Default + 'static {
    /// Container color.
    fn container_color() -> Color;

    /// Content color.
    fn content_color() -> Color;
}

/// Surface FAB color tokens.
#[derive(Debug, Clone, Copy, Default)]
pub struct SurfaceFab;

/// Primary FAB color tokens.
#[derive(Debug, Clone, Copy, Default)]
pub struct PrimaryFab;

/// Secondary FAB color tokens.
#[derive(Debug, Clone, Copy, Default)]
pub struct SecondaryFab;

/// Tertiary FAB color tokens.
#[derive(Debug, Clone, Copy, Default)]
pub struct TertiaryFab;

impl FabVariantTokens for SurfaceFab {
    fn container_color() -> Color {
        SurfaceContainerHigh.into()
    }

    fn content_color() -> Color {
        Primary.into()
    }
}

impl FabVariantTokens for PrimaryFab {
    fn container_color() -> Color {
        PrimaryContainer.into()
    }

    fn content_color() -> Color {
        OnPrimaryContainer.into()
    }
}

impl FabVariantTokens for SecondaryFab {
    fn container_color() -> Color {
        SecondaryContainer.into()
    }

    fn content_color() -> Color {
        OnSecondaryContainer.into()
    }
}

impl FabVariantTokens for TertiaryFab {
    fn container_color() -> Color {
        TertiaryContainer.into()
    }

    fn content_color() -> Color {
        OnTertiaryContainer.into()
    }
}

/// A Material Design 3 floating action button.
pub struct Fab<Content, Action = fn(&Environment), Tokens = SurfaceFab> {
    accessibility_label: Str,
    content: Content,
    action: Action,
    tokens: PhantomData<Tokens>,
}

impl<Content, Action, Tokens> Debug for Fab<Content, Action, Tokens> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fab")
            .field("accessibility_label", &self.accessibility_label)
            .finish_non_exhaustive()
    }
}

impl<Content> Fab<Content, fn(&Environment), SurfaceFab> {
    /// Creates a surface FAB with arbitrary visual content.
    #[must_use]
    pub fn new(accessibility_label: impl Into<Str>, content: Content) -> Self {
        Self {
            accessibility_label: accessibility_label.into(),
            content,
            action: noop,
            tokens: PhantomData,
        }
    }
}

impl<Content, Action, Tokens> Fab<Content, Action, Tokens> {
    /// Uses primary FAB color tokens.
    #[must_use]
    pub fn primary(self) -> Fab<Content, Action, PrimaryFab> {
        self.with_variant()
    }

    /// Uses secondary FAB color tokens.
    #[must_use]
    pub fn secondary(self) -> Fab<Content, Action, SecondaryFab> {
        self.with_variant()
    }

    /// Uses tertiary FAB color tokens.
    #[must_use]
    pub fn tertiary(self) -> Fab<Content, Action, TertiaryFab> {
        self.with_variant()
    }

    fn with_variant<NewTokens>(self) -> Fab<Content, Action, NewTokens> {
        Fab {
            accessibility_label: self.accessibility_label,
            content: self.content,
            action: self.action,
            tokens: PhantomData,
        }
    }

    /// Sets the action performed when the FAB is tapped.
    #[must_use]
    pub fn action<F, Args>(self, action: F) -> Fab<Content, impl FnMut(&Environment), Tokens>
    where
        F: Handler<Args, ()> + 'static,
    {
        Fab {
            accessibility_label: self.accessibility_label,
            content: self.content,
            action: boxed_action(action),
            tokens: PhantomData,
        }
    }
}

impl<Content, Action, Tokens> View for Fab<Content, Action, Tokens>
where
    Content: View + 'static,
    Action: FnMut(&Environment) + 'static,
    Tokens: FabVariantTokens,
{
    fn body(self, _env: &Environment) -> impl View {
        let mut action = self.action;

        self.content
            .foreground(Tokens::content_color())
            .size(FAB_ICON_SIZE, FAB_ICON_SIZE)
            .padding_with(16.0)
            .size(FAB_CONTAINER_WIDTH, FAB_CONTAINER_HEIGHT)
            .background(
                RoundedRectangle::new(FAB_CONTAINER_CLIP_RADIUS).fill(Tokens::container_color()),
            )
            .background(fab_elevation(FAB_CONTAINER_CLIP_RADIUS))
            .on_tap(move |env: Environment| action(&env))
            .a11y_label(self.accessibility_label)
            .a11y_role(AccessibilityRole::Button)
            .a11y_children(AccessibilityChildren::ExcludeDescendants)
            .install(interaction_style(
                Tokens::content_color(),
                f64::from(FAB_CONTAINER_SHAPE),
            ))
    }
}

/// A Material Design 3 extended floating action button.
pub struct ExtendedFab<Action = fn(&Environment), Tokens = PrimaryFab> {
    label: Label,
    accessibility_label: Str,
    action: Action,
    tokens: PhantomData<Tokens>,
}

impl<Action, Tokens> Debug for ExtendedFab<Action, Tokens> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExtendedFab")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl ExtendedFab<fn(&Environment), PrimaryFab> {
    /// Creates a primary extended FAB with a semantic text label.
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
            tokens: PhantomData,
        }
    }
}

impl<Action, Tokens> ExtendedFab<Action, Tokens> {
    /// Uses surface FAB color tokens.
    #[must_use]
    pub fn surface(self) -> ExtendedFab<Action, SurfaceFab> {
        self.with_variant()
    }

    /// Uses secondary FAB color tokens.
    #[must_use]
    pub fn secondary(self) -> ExtendedFab<Action, SecondaryFab> {
        self.with_variant()
    }

    /// Uses tertiary FAB color tokens.
    #[must_use]
    pub fn tertiary(self) -> ExtendedFab<Action, TertiaryFab> {
        self.with_variant()
    }

    fn with_variant<NewTokens>(self) -> ExtendedFab<Action, NewTokens> {
        ExtendedFab {
            label: self.label,
            accessibility_label: self.accessibility_label,
            action: self.action,
            tokens: PhantomData,
        }
    }

    /// Sets the action performed when the extended FAB is tapped.
    #[must_use]
    pub fn action<F, Args>(self, action: F) -> ExtendedFab<impl FnMut(&Environment), Tokens>
    where
        F: Handler<Args, ()> + 'static,
    {
        ExtendedFab {
            label: self.label,
            accessibility_label: self.accessibility_label,
            action: boxed_action(action),
            tokens: PhantomData,
        }
    }
}

impl<Action, Tokens> View for ExtendedFab<Action, Tokens>
where
    Action: FnMut(&Environment) + 'static,
    Tokens: FabVariantTokens,
{
    fn body(self, _env: &Environment) -> impl View {
        let mut action = self.action;

        self.label
            .font(typography::label_large())
            .foreground(Tokens::content_color())
            .height(EXTENDED_FAB_HEIGHT)
            .padding_with(EdgeInsets::new(
                0.0,
                0.0,
                EXTENDED_FAB_LEADING_SPACE_WITHOUT_ICON,
                EXTENDED_FAB_TRAILING_SPACE,
            ))
            .background(
                RoundedRectangle::new(EXTENDED_FAB_CLIP_RADIUS).fill(Tokens::container_color()),
            )
            .background(fab_elevation(EXTENDED_FAB_CLIP_RADIUS))
            .on_tap(move |env: Environment| action(&env))
            .a11y_label(self.accessibility_label)
            .a11y_role(AccessibilityRole::Button)
            .a11y_children(AccessibilityChildren::ExcludeDescendants)
            .install(interaction_style(
                Tokens::content_color(),
                f64::from(EXTENDED_FAB_SHAPE),
            ))
    }
}

const fn noop(_env: &Environment) {}

fn fab_elevation(clip_radius: f32) -> impl View {
    zstack((
        RoundedRectangle::new(clip_radius)
            .fill(ShadowColor.with_opacity(FAB_ELEVATION_AMBIENT_OPACITY))
            .offset(0.0, FAB_ELEVATION_AMBIENT_OFFSET_Y),
        RoundedRectangle::new(clip_radius)
            .fill(ShadowColor.with_opacity(FAB_ELEVATION_KEY_OPACITY))
            .offset(0.0, FAB_ELEVATION_KEY_OFFSET_Y),
    ))
}

/// Creates a surface FAB with arbitrary visual content.
#[must_use]
pub fn fab<Content>(accessibility_label: impl Into<Str>, content: Content) -> Fab<Content>
where
    Content: View + 'static,
{
    Fab::new(accessibility_label, content)
}

/// Creates a primary extended FAB with a semantic text label.
#[must_use]
pub fn extended_fab(label: impl IntoLabel) -> ExtendedFab {
    ExtendedFab::new(label)
}

#[cfg(test)]
mod tests {
    use super::{
        EXTENDED_FAB_HEIGHT, EXTENDED_FAB_LEADING_SPACE_WITHOUT_ICON, EXTENDED_FAB_SHAPE,
        EXTENDED_FAB_TRAILING_SPACE, FAB_CONTAINER_HEIGHT, FAB_CONTAINER_SHAPE,
        FAB_CONTAINER_WIDTH, FAB_ICON_SIZE,
    };

    #[test]
    fn fab_tokens_match_material_web_v0_192_surface_fab() {
        assert_eq!(FAB_CONTAINER_HEIGHT, 56.0);
        assert_eq!(FAB_CONTAINER_WIDTH, 56.0);
        assert_eq!(FAB_CONTAINER_SHAPE, 16.0);
        assert_eq!(FAB_ICON_SIZE, 24.0);
    }

    #[test]
    fn extended_fab_tokens_match_material_web_v0_192_primary_extended_fab() {
        assert_eq!(EXTENDED_FAB_HEIGHT, 56.0);
        assert_eq!(EXTENDED_FAB_SHAPE, 16.0);
        assert_eq!(EXTENDED_FAB_LEADING_SPACE_WITHOUT_ICON, 20.0);
        assert_eq!(EXTENDED_FAB_TRAILING_SPACE, 20.0);
    }
}
