//! Material Design 3 floating action buttons composed from `WaterUI` primitives.

use core::fmt::{self, Debug};
use core::marker::PhantomData;

use waterui::accessibility::{AccessibilityChildren, AccessibilityRole};
use waterui::color::Color;
use waterui::layout::padding::EdgeInsets;
use waterui::style::FloatingStyle;
use waterui::{Environment, Signal, Str, View, ViewExt as _};
use waterui_controls::label::{IntoLabel, Label};
use waterui_core::handler::{Handler, boxed_action};

use crate::color::{
    OnPrimaryContainer, OnSecondaryContainer, OnTertiaryContainer, Primary, PrimaryContainer,
    SecondaryContainer, SurfaceContainerHigh, TertiaryContainer,
};
use crate::elevation::{MaterialElevationLevel, apply_to_floating_style};
use crate::semantics::interaction_style;
use crate::theme::typography;

/// The geometry of one FAB size.
///
/// Material 3 Expressive offers three, and they differ in corner shape as well
/// as in scale, so a size is a token set rather than a diameter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FabSizeTokens {
    /// `ContainerWidth` / `ContainerHeight`.
    pub container: f32,
    /// `IconSize`.
    pub icon: f32,
    /// `ContainerShape`, as a corner radius.
    pub corner_radius: f32,
}

impl FabSizeTokens {
    /// The corner radius as a fraction of the container, for clip shapes.
    #[must_use]
    pub fn clip_radius(self) -> f32 {
        self.corner_radius / self.container
    }

    /// The inset that centres the icon in the container.
    #[must_use]
    pub fn icon_padding(self) -> f32 {
        (self.container - self.icon) / 2.0
    }
}

/// How large a floating action button is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FabSize {
    /// `FabBaselineTokens`: the standard 56dp FAB.
    #[default]
    Baseline,
    /// `FabMediumTokens`.
    Medium,
    /// `FabLargeTokens`.
    Large,
}

impl FabSize {
    /// The token set for this size.
    #[must_use]
    pub const fn tokens(self) -> FabSizeTokens {
        match self {
            // FabBaselineTokens: CornerLarge
            Self::Baseline => FabSizeTokens {
                container: 56.0,
                icon: 24.0,
                corner_radius: 16.0,
            },
            // FabMediumTokens: CornerLargeIncreased
            Self::Medium => FabSizeTokens {
                container: 80.0,
                icon: 28.0,
                corner_radius: 20.0,
            },
            // FabLargeTokens: CornerExtraLarge
            Self::Large => FabSizeTokens {
                container: 96.0,
                icon: 32.0,
                corner_radius: 28.0,
            },
        }
    }
}

const EXTENDED_FAB_HEIGHT: f32 = 56.0;
const EXTENDED_FAB_SHAPE: f32 = 16.0;
const EXTENDED_FAB_CLIP_RADIUS: f32 = EXTENDED_FAB_SHAPE / EXTENDED_FAB_HEIGHT;
const EXTENDED_FAB_LEADING_SPACE_WITHOUT_ICON: f32 = 20.0;
const EXTENDED_FAB_TRAILING_SPACE: f32 = 20.0;

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
    size: FabSize,
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
            size: FabSize::default(),
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
            size: self.size,
            tokens: PhantomData,
        }
    }

    /// Sets how large the FAB is drawn.
    #[must_use]
    pub const fn size(mut self, size: FabSize) -> Self {
        self.size = size;
        self
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
            size: self.size,
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
        let size = self.size.tokens();
        let floating_style = floating_style::<Tokens>(
            f64::from(size.container),
            f64::from(size.container),
            size.clip_radius(),
        );

        self.content
            .foreground(Tokens::content_color())
            .size(size.icon, size.icon)
            .padding_with(size.icon_padding())
            .size(size.container, size.container)
            .floating_with(floating_style)
            .on_tap(move |env: Environment| action(&env))
            .a11y_label(self.accessibility_label)
            .a11y_role(AccessibilityRole::Button)
            .a11y_children(AccessibilityChildren::ExcludeDescendants)
            .install(interaction_style(
                Tokens::content_color(),
                f64::from(size.corner_radius),
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
        let floating_style = floating_style::<Tokens>(
            0.0,
            f64::from(EXTENDED_FAB_HEIGHT),
            EXTENDED_FAB_CLIP_RADIUS,
        );

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
            .floating_with(floating_style)
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

pub(crate) fn theme() -> FloatingStyle {
    let size = FabSize::Baseline.tokens();
    floating_style::<SurfaceFab>(
        f64::from(size.container),
        f64::from(size.container),
        size.clip_radius(),
    )
}

fn floating_style<Tokens>(
    minimum_width: f64,
    minimum_height: f64,
    clip_radius: f32,
) -> FloatingStyle
where
    Tokens: FabVariantTokens,
{
    let mut style = FloatingStyle {
        container_color: Tokens::container_color(),
        content_color: Tokens::content_color(),
        state_layer_color: Tokens::content_color(),
        clip_radius,
        content_inset_x: 0.0,
        content_inset_y: 0.0,
        minimum_width,
        minimum_height,
        disabled_content_opacity: 0.38,
        ..FloatingStyle::default()
    };
    apply_to_floating_style(&mut style, MaterialElevationLevel::LEVEL3);
    style
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
        EXTENDED_FAB_TRAILING_SPACE, FabSize,
    };

    /// `FabBaselineTokens`, `FabMediumTokens` and `FabLargeTokens`. The corner
    /// shape grows with the container — `CornerLarge`, `CornerLargeIncreased`,
    /// `CornerExtraLarge` — so a larger FAB is not simply a scaled-up one.
    #[test]
    fn fab_sizes_match_compose_fab_tokens() {
        let baseline = FabSize::Baseline.tokens();
        assert_eq!(baseline.container, 56.0);
        assert_eq!(baseline.icon, 24.0);
        assert_eq!(baseline.corner_radius, 16.0);

        let medium = FabSize::Medium.tokens();
        assert_eq!(medium.container, 80.0);
        assert_eq!(medium.icon, 28.0);
        assert_eq!(medium.corner_radius, 20.0);

        let large = FabSize::Large.tokens();
        assert_eq!(large.container, 96.0);
        assert_eq!(large.icon, 32.0);
        assert_eq!(large.corner_radius, 28.0);

        // The icon stays centred at every size.
        for size in [baseline, medium, large] {
            assert!((size.icon_padding().mul_add(2.0, size.icon) - size.container).abs() < 1e-6);
            assert!(size.clip_radius() > 0.0 && size.clip_radius() < 0.5);
        }
    }

    #[test]
    fn extended_fab_tokens_match_compose_extended_fab_tokens() {
        assert_eq!(EXTENDED_FAB_HEIGHT, 56.0);
        assert_eq!(EXTENDED_FAB_SHAPE, 16.0);
        assert_eq!(EXTENDED_FAB_LEADING_SPACE_WITHOUT_ICON, 20.0);
        assert_eq!(EXTENDED_FAB_TRAILING_SPACE, 20.0);
    }
}
