//! Material Design 3 icon buttons composed from `WaterUI` primitives.

use core::fmt::{self, Debug};
use core::marker::PhantomData;

use waterui::accessibility::{AccessibilityChildren, AccessibilityRole};
use waterui::border::Border;
use waterui::color::Color;
use waterui::shape::{Circle, ShapeExt as _};
use waterui::{Environment, Str, View, ViewExt as _};
use waterui_core::handler::{Handler, boxed_action};

use crate::color::{
    InverseOnSurface, InverseSurface, OnPrimary, OnSecondaryContainer, OnSurfaceVariant, Outline,
    Primary, SecondaryContainer,
};
use crate::semantics::interaction_style;

/// `IconButtonTokens.StateLayerSize`: the size of the visible state layer.
const ICON_BUTTON_STATE_LAYER_SIZE: f32 = 40.0;
/// `IconButtonTokens.IconSize`.
const ICON_BUTTON_ICON_SIZE: f32 = 24.0;
/// Compose wraps every icon button in `minimumInteractiveComponentSize()`,
/// documented as "an overall minimum touch target size of 48 x 48dp, to meet
/// accessibility guidelines". Compose can expand the touch target without
/// growing the layout; `WaterUI` has no such split, so the button occupies the
/// full target and draws its 40dp state layer centered inside.
const ICON_BUTTON_TOUCH_TARGET_SIZE: f32 = 48.0;
const ICON_BUTTON_OUTLINE_WIDTH: f32 = 1.0;

/// The geometry of one icon-button size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IconButtonSizeTokens {
    /// The box the button occupies, which is also its hit area.
    pub container: f32,
    /// The state layer drawn inside that box.
    pub state_layer: f32,
    /// `IconSize`.
    pub icon: f32,
    /// `OutlinedOutlineWidth`.
    pub outline_width: f32,
}

/// How large an icon button is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum IconButtonSize {
    /// The standard icon button: a 40dp state layer inside the 48dp target.
    #[default]
    Small,
    /// `MediumIconButtonTokens`.
    Medium,
    /// `LargeIconButtonTokens`.
    Large,
}

impl IconButtonSize {
    /// The token set for this size.
    #[must_use]
    pub const fn tokens(self) -> IconButtonSizeTokens {
        match self {
            Self::Small => IconButtonSizeTokens {
                container: ICON_BUTTON_TOUCH_TARGET_SIZE,
                state_layer: ICON_BUTTON_STATE_LAYER_SIZE,
                icon: ICON_BUTTON_ICON_SIZE,
                outline_width: ICON_BUTTON_OUTLINE_WIDTH,
            },
            // MediumIconButtonTokens. At this size the container *is* the
            // state layer — it is already well past the 48dp minimum.
            Self::Medium => IconButtonSizeTokens {
                container: 56.0,
                state_layer: 56.0,
                icon: 24.0,
                outline_width: 1.0,
            },
            // LargeIconButtonTokens.
            Self::Large => IconButtonSizeTokens {
                container: 96.0,
                state_layer: 96.0,
                icon: 32.0,
                outline_width: 2.0,
            },
        }
    }
}

/// Color token set for an icon button variant.
pub trait IconButtonVariantTokens: Default + 'static {
    /// Container color.
    fn container_color() -> Color;

    /// Icon/content color.
    fn icon_color() -> Color;

    /// Border color.
    #[must_use]
    fn outline_color() -> Color {
        Outline.into()
    }

    /// Border width.
    #[must_use]
    fn outline_width() -> f32 {
        0.0
    }
}

/// Standard icon button tokens.
#[derive(Debug, Clone, Copy, Default)]
pub struct StandardIconButton;

/// Filled icon button tokens.
#[derive(Debug, Clone, Copy, Default)]
pub struct FilledIconButton;

/// Filled tonal icon button tokens.
#[derive(Debug, Clone, Copy, Default)]
pub struct FilledTonalIconButton;

/// Outlined icon button tokens.
#[derive(Debug, Clone, Copy, Default)]
pub struct OutlinedIconButton;

impl IconButtonVariantTokens for StandardIconButton {
    fn container_color() -> Color {
        crate::color::Surface.with_opacity(0.0).into()
    }

    fn icon_color() -> Color {
        OnSurfaceVariant.into()
    }
}

impl IconButtonVariantTokens for FilledIconButton {
    fn container_color() -> Color {
        Primary.into()
    }

    fn icon_color() -> Color {
        OnPrimary.into()
    }
}

impl IconButtonVariantTokens for FilledTonalIconButton {
    fn container_color() -> Color {
        SecondaryContainer.into()
    }

    fn icon_color() -> Color {
        OnSecondaryContainer.into()
    }
}

impl IconButtonVariantTokens for OutlinedIconButton {
    fn container_color() -> Color {
        crate::color::Surface.with_opacity(0.0).into()
    }

    fn icon_color() -> Color {
        OnSurfaceVariant.into()
    }

    fn outline_width() -> f32 {
        ICON_BUTTON_OUTLINE_WIDTH
    }
}

/// Selected standard icon button tokens.
#[derive(Debug, Clone, Copy, Default)]
pub struct SelectedStandardIconButton;

/// Selected outlined icon button tokens.
#[derive(Debug, Clone, Copy, Default)]
pub struct SelectedOutlinedIconButton;

impl IconButtonVariantTokens for SelectedStandardIconButton {
    fn container_color() -> Color {
        crate::color::Surface.with_opacity(0.0).into()
    }

    fn icon_color() -> Color {
        Primary.into()
    }
}

impl IconButtonVariantTokens for SelectedOutlinedIconButton {
    fn container_color() -> Color {
        InverseSurface.into()
    }

    fn icon_color() -> Color {
        InverseOnSurface.into()
    }
}

/// A Material Design 3 icon button.
pub struct IconButton<Content, Action = fn(&Environment), Tokens = StandardIconButton> {
    accessibility_label: Str,
    content: Content,
    action: Action,
    size: IconButtonSize,
    tokens: PhantomData<Tokens>,
}

impl<Content, Action, Tokens> Debug for IconButton<Content, Action, Tokens> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IconButton")
            .field("accessibility_label", &self.accessibility_label)
            .finish_non_exhaustive()
    }
}

impl<Content> IconButton<Content, fn(&Environment), StandardIconButton> {
    /// Creates a standard icon button with arbitrary visual icon content.
    #[must_use]
    pub fn new(accessibility_label: impl Into<Str>, content: Content) -> Self {
        Self {
            accessibility_label: accessibility_label.into(),
            content,
            action: noop,
            size: IconButtonSize::default(),
            tokens: PhantomData,
        }
    }
}

impl<Content, Action, Tokens> IconButton<Content, Action, Tokens> {
    /// Uses filled icon button tokens.
    #[must_use]
    pub fn filled(self) -> IconButton<Content, Action, FilledIconButton> {
        self.with_variant()
    }

    /// Uses filled tonal icon button tokens.
    #[must_use]
    pub fn filled_tonal(self) -> IconButton<Content, Action, FilledTonalIconButton> {
        self.with_variant()
    }

    /// Uses outlined icon button tokens.
    #[must_use]
    pub fn outlined(self) -> IconButton<Content, Action, OutlinedIconButton> {
        self.with_variant()
    }

    /// Uses selected standard icon button tokens.
    #[must_use]
    pub fn selected(self) -> IconButton<Content, Action, SelectedStandardIconButton> {
        self.with_variant()
    }

    /// Uses selected outlined icon button tokens.
    #[must_use]
    pub fn selected_outlined(self) -> IconButton<Content, Action, SelectedOutlinedIconButton> {
        self.with_variant()
    }

    fn with_variant<NewTokens>(self) -> IconButton<Content, Action, NewTokens> {
        IconButton {
            accessibility_label: self.accessibility_label,
            content: self.content,
            action: self.action,
            size: self.size,
            tokens: PhantomData,
        }
    }

    /// Sets how large the icon button is drawn.
    #[must_use]
    pub const fn size(mut self, size: IconButtonSize) -> Self {
        self.size = size;
        self
    }

    /// Sets the action performed when the icon button is tapped.
    #[must_use]
    pub fn action<F, Args>(self, action: F) -> IconButton<Content, impl FnMut(&Environment), Tokens>
    where
        F: Handler<Args, ()> + 'static,
    {
        IconButton {
            accessibility_label: self.accessibility_label,
            content: self.content,
            action: boxed_action(action),
            size: self.size,
            tokens: PhantomData,
        }
    }
}

impl<Content, Action, Tokens> View for IconButton<Content, Action, Tokens>
where
    Content: View + 'static,
    Action: FnMut(&Environment) + 'static,
    Tokens: IconButtonVariantTokens,
{
    fn body(self, _env: &Environment) -> impl View {
        let mut action = self.action;
        let size = self.size.tokens();

        self.content
            .foreground(Tokens::icon_color())
            .size(size.icon, size.icon)
            .padding_with((size.state_layer - size.icon) * 0.5)
            .size(size.state_layer, size.state_layer)
            .background(Circle.fill(Tokens::container_color()))
            .border_with(
                Border::new(Tokens::outline_color(), Tokens::outline_width())
                    .corner_radius(size.state_layer * 0.5),
            )
            .size(size.container, size.container)
            .on_tap(move |env: Environment| action(&env))
            .a11y_label(self.accessibility_label)
            .a11y_role(AccessibilityRole::Button)
            .a11y_children(AccessibilityChildren::ExcludeDescendants)
            .install(interaction_style(
                Tokens::icon_color(),
                f64::from(size.state_layer * 0.5),
            ))
    }
}

const fn noop(_env: &Environment) {}

/// Creates a standard icon button with arbitrary visual icon content.
#[must_use]
pub fn icon_button<Content>(
    accessibility_label: impl Into<Str>,
    content: Content,
) -> IconButton<Content>
where
    Content: View + 'static,
{
    IconButton::new(accessibility_label, content)
}

/// Creates a filled icon button with arbitrary visual icon content.
#[must_use]
pub fn filled_icon_button<Content>(
    accessibility_label: impl Into<Str>,
    content: Content,
) -> IconButton<Content, fn(&Environment), FilledIconButton>
where
    Content: View + 'static,
{
    icon_button(accessibility_label, content).filled()
}

/// Creates a filled tonal icon button with arbitrary visual icon content.
#[must_use]
pub fn filled_tonal_icon_button<Content>(
    accessibility_label: impl Into<Str>,
    content: Content,
) -> IconButton<Content, fn(&Environment), FilledTonalIconButton>
where
    Content: View + 'static,
{
    icon_button(accessibility_label, content).filled_tonal()
}

/// Creates an outlined icon button with arbitrary visual icon content.
#[must_use]
pub fn outlined_icon_button<Content>(
    accessibility_label: impl Into<Str>,
    content: Content,
) -> IconButton<Content, fn(&Environment), OutlinedIconButton>
where
    Content: View + 'static,
{
    icon_button(accessibility_label, content).outlined()
}

#[cfg(test)]
mod tests {
    use super::{
        ICON_BUTTON_ICON_SIZE, ICON_BUTTON_OUTLINE_WIDTH, ICON_BUTTON_STATE_LAYER_SIZE,
        IconButtonSize, IconButtonVariantTokens, OutlinedIconButton,
    };

    /// `MediumIconButtonTokens` and `LargeIconButtonTokens`. Past the small
    /// size the container has outgrown the 48dp minimum, so the state layer
    /// fills it rather than sitting inside a larger touch target.
    #[test]
    fn icon_button_size_scale_matches_compose_icon_button_tokens() {
        let small = IconButtonSize::Small.tokens();
        assert_eq!(small.container, 48.0);
        assert_eq!(small.state_layer, 40.0);
        assert_eq!(small.icon, 24.0);

        let medium = IconButtonSize::Medium.tokens();
        assert_eq!(medium.container, 56.0);
        assert_eq!(medium.icon, 24.0);
        assert_eq!(medium.outline_width, 1.0);

        let large = IconButtonSize::Large.tokens();
        assert_eq!(large.container, 96.0);
        assert_eq!(large.icon, 32.0);
        assert_eq!(large.outline_width, 2.0);

        // Every size clears the 48dp minimum touch target, and no state layer
        // ever spills outside the box that receives the taps.
        for size in [small, medium, large] {
            assert!(size.container >= 48.0);
            assert!(size.state_layer <= size.container);
            assert!(size.icon < size.state_layer);
        }
    }

    #[test]
    fn icon_button_tokens_match_compose_icon_button_tokens() {
        assert_eq!(ICON_BUTTON_STATE_LAYER_SIZE, 40.0);
        assert_eq!(ICON_BUTTON_ICON_SIZE, 24.0);
    }

    #[test]
    fn outlined_icon_button_tokens_match_compose_icon_button_tokens() {
        assert_eq!(
            OutlinedIconButton::outline_width(),
            ICON_BUTTON_OUTLINE_WIDTH
        );
    }
}
