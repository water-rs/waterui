//! A card is a styled container that groups related content.
use crate::prelude::*;
use crate::style::{Shadow, Vector};
use waterui_graphics::color::Color;
use waterui_layout::stack::vstack;
use waterui_shape::{RoundedRectangle, ShapeExt};
use waterui_text::{IntoText, font::Title};

/// Visual style for a [`Card`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum CardStyle {
    /// Use the card style provided by the installed theme.
    #[default]
    Automatic,
    /// Elevated card style.
    Elevated,
    /// Filled card style.
    Filled,
    /// Outlined card style.
    Outlined,
}

/// Theme tokens used by the Rust-side [`Card`] composer.
#[derive(Debug, Clone)]
pub struct CardTheme {
    /// Default card style.
    pub default_style: CardStyle,
    /// Elevated card tokens.
    pub elevated: CardStyleTokens,
    /// Filled card tokens.
    pub filled: CardStyleTokens,
    /// Outlined card tokens.
    pub outlined: CardStyleTokens,
    /// Inner content padding.
    pub content_padding: f32,
    /// Gap between title, subtitle, and content.
    pub content_spacing: f32,
}

/// Per-style card tokens.
#[derive(Debug, Clone)]
pub struct CardStyleTokens {
    /// Container fill color.
    pub container_color: Color,
    /// Border color.
    pub outline_color: Color,
    /// Border width.
    pub outline_width: f32,
    /// Corner radius in logical units for borders.
    pub corner_radius: f32,
    /// Normalized corner radius for clipping.
    pub clip_radius: f32,
    /// Shadow color.
    pub shadow_color: Color,
    /// Shadow blur radius.
    pub shadow_radius: f32,
    /// Shadow vertical offset.
    pub shadow_offset_y: f32,
}

impl CardTheme {
    /// Returns the tokens for a card style.
    #[must_use]
    pub fn tokens(&self, style: CardStyle) -> &CardStyleTokens {
        match match style {
            CardStyle::Automatic => self.default_style,
            other => other,
        } {
            CardStyle::Automatic => &self.filled,
            CardStyle::Elevated => &self.elevated,
            CardStyle::Filled => &self.filled,
            CardStyle::Outlined => &self.outlined,
        }
    }
}

/// A card widget that displays optional title, subtitle, and content.
#[derive(Debug, Clone)]
pub struct Card<Content> {
    title: Option<Text>,
    subtitle: Option<Text>,
    content: Content,
    style: CardStyle,
}

impl<Content> Card<Content> {
    // Creates a new card with the specified content.

    /// # Arguments
    /// * `content` - The main content of the card.
    pub const fn new(content: Content) -> Self {
        Self {
            title: None,
            subtitle: None,
            content,
            style: CardStyle::Automatic,
        }
    }
    /// Sets the title of the card.
    #[must_use]
    pub fn title(mut self, title: impl IntoText) -> Self {
        self.title = Some(title.into_text().font(Title));
        self
    }

    /// Sets the subtitle of the card.
    #[must_use]
    pub fn subtitle(mut self, subtitle: impl IntoText) -> Self {
        self.subtitle = Some(subtitle.into_text());
        self
    }

    /// Sets the card visual style.
    #[must_use]
    pub const fn style(mut self, style: CardStyle) -> Self {
        self.style = style;
        self
    }
}

impl<Content> View for Card<Content>
where
    Content: View,
{
    fn body(self, env: &Environment) -> impl View {
        let Some(theme) = env.get::<CardTheme>() else {
            return AnyView::new(vstack((self.title, self.subtitle, self.content)));
        };
        let tokens = theme.tokens(self.style);
        let shadow = Shadow::new(
            tokens.shadow_color.clone(),
            Vector::new(0.0, tokens.shadow_offset_y),
            tokens.shadow_radius,
        );
        AnyView::new(
            vstack((self.title, self.subtitle, self.content))
                .spacing(theme.content_spacing)
                .padding_with(theme.content_padding)
                .background(
                    RoundedRectangle::new(tokens.clip_radius).fill(tokens.container_color.clone()),
                )
                .border_with(
                    Border::new(tokens.outline_color.clone(), tokens.outline_width)
                        .corner_radius(tokens.corner_radius),
                )
                .shadow(shadow),
        )
    }
}

/// Creates a new card widget with the specified content.
///
/// # Arguments
/// * `content` - The main content of the card.
pub const fn card<Content>(content: Content) -> Card<Content> {
    Card::new(content)
}
