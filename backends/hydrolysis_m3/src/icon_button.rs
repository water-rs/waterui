//! Material Design 3 icon buttons composed from WaterUI primitives.

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

const ICON_BUTTON_STATE_LAYER_SIZE: f32 = 40.0;
const ICON_BUTTON_ICON_SIZE: f32 = 24.0;
const ICON_BUTTON_OUTLINE_WIDTH: f32 = 1.0;

/// Color token set for an icon button variant.
pub trait IconButtonVariantTokens: Default + 'static {
    /// Container color.
    fn container_color() -> Color;

    /// Icon/content color.
    fn icon_color() -> Color;

    /// Border color.
    fn outline_color() -> Color {
        Outline.into()
    }

    /// Border width.
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
            tokens: PhantomData,
        }
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

        self.content
            .foreground(Tokens::icon_color())
            .size(ICON_BUTTON_ICON_SIZE, ICON_BUTTON_ICON_SIZE)
            .padding_with((ICON_BUTTON_STATE_LAYER_SIZE - ICON_BUTTON_ICON_SIZE) * 0.5)
            .size(ICON_BUTTON_STATE_LAYER_SIZE, ICON_BUTTON_STATE_LAYER_SIZE)
            .background(Circle.fill(Tokens::container_color()))
            .border_with(
                Border::new(Tokens::outline_color(), Tokens::outline_width())
                    .corner_radius(ICON_BUTTON_STATE_LAYER_SIZE * 0.5),
            )
            .on_tap(move |env: Environment| action(&env))
            .a11y_label(self.accessibility_label)
            .a11y_role(AccessibilityRole::Button)
            .a11y_children(AccessibilityChildren::ExcludeDescendants)
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
        IconButtonVariantTokens, OutlinedIconButton,
    };

    #[test]
    fn icon_button_tokens_match_material_web_v0_192() {
        assert_eq!(ICON_BUTTON_STATE_LAYER_SIZE, 40.0);
        assert_eq!(ICON_BUTTON_ICON_SIZE, 24.0);
    }

    #[test]
    fn outlined_icon_button_tokens_match_material_web_v0_192() {
        assert_eq!(
            OutlinedIconButton::outline_width(),
            ICON_BUTTON_OUTLINE_WIDTH
        );
    }
}
