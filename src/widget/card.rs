//! A card is a styled container that groups related content.

use core::fmt;

use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::{AnyView, Environment, Native, NativeView, View};
use waterui_graphics::color::Color;
use waterui_text::{IntoText, Text};

/// Visual style for a [`Card`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
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

/// Resolved configuration passed to [`NativeBackend`](crate::Native) for a [`Card`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CardConfig<S> {
    /// Optional title displayed at the top of the card.
    pub title: Option<Text>,
    /// Optional subtitle displayed below the title.
    pub subtitle: Option<Text>,
    /// Main content of the card.
    pub content: S,
    /// Semantic visual style for the card.
    pub style: CardStyle,
}

impl<S> CardConfig<S> {
    /// Creates a new card configuration.
    #[must_use]
    pub const fn new(
        title: Option<Text>,
        subtitle: Option<Text>,
        content: S,
        style: CardStyle,
    ) -> Self {
        Self {
            title,
            subtitle,
            content,
            style,
        }
    }
}

impl<S: 'static> NativeView for CardConfig<S> {}

impl<S: 'static> MapSlots<S> for CardConfig<S> {
    type With<T: 'static> = CardConfig<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> CardConfig<T> {
        CardConfig {
            title: self.title,
            subtitle: self.subtitle,
            content: f(self.content),
            style: self.style,
        }
    }
}

impl<S: 'static> fmt::Display for CardConfig<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Card")
    }
}

/// Legacy theme tokens kept hidden while downstream style packages migrate to
/// `WidgetTheme::card`.
#[doc(hidden)]
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

/// Legacy per-style card tokens kept hidden while downstream style packages
/// migrate to `WidgetTheme::card`.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct CardStyleTokens {
    /// Container fill color.
    pub container_color: Color,
    /// Border color.
    pub outline_color: Color,
    /// Border width.
    pub outline_width: f32,
    /// Corner radius in logical units.
    pub corner_radius: f32,
    /// Shadow color.
    pub shadow_color: Color,
    /// Shadow blur radius.
    pub shadow_radius: f32,
    /// Shadow vertical offset.
    pub shadow_offset_y: f32,
    /// Ambient shadow color.
    pub ambient_shadow_color: Color,
    /// Ambient shadow blur radius.
    pub ambient_shadow_radius: f32,
    /// Ambient shadow vertical offset.
    pub ambient_shadow_offset_y: f32,
}

impl CardTheme {
    /// Returns the tokens for a card style.
    #[must_use]
    pub const fn tokens(&self, style: CardStyle) -> &CardStyleTokens {
        match match style {
            CardStyle::Automatic => self.default_style,
            other => other,
        } {
            CardStyle::Automatic | CardStyle::Filled => &self.filled,
            CardStyle::Elevated => &self.elevated,
            CardStyle::Outlined => &self.outlined,
        }
    }
}

/// A card widget that displays optional title, subtitle, and content.
#[derive(Debug, Clone)]
#[must_use]
pub struct Card<Content> {
    title: Option<Text>,
    subtitle: Option<Text>,
    content: Content,
    style: CardStyle,
}

impl<Content: View> Card<Content> {
    /// Creates a new card with the specified content.
    ///
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
    pub fn title(mut self, title: impl IntoText) -> Self {
        self.title = Some(title.into_text());
        self
    }

    /// Sets the subtitle of the card.
    pub fn subtitle(mut self, subtitle: impl IntoText) -> Self {
        self.subtitle = Some(subtitle.into_text());
        self
    }

    /// Sets the card visual style.
    pub const fn style(mut self, style: CardStyle) -> Self {
        self.style = style;
        self
    }

    fn into_config<S: ChildSlot>(self) -> CardConfig<S> {
        CardConfig {
            title: self.title,
            subtitle: self.subtitle,
            content: S::from_view(self.content),
            style: self.style,
        }
    }
}

impl<Content> View for Card<Content>
where
    Content: View + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self.into_config::<AnyView>())
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self.into_config::<B::Slot>(), env, cx)
    }
}

/// Creates a new card widget with the specified content.
///
/// # Arguments
/// * `content` - The main content of the card.
pub const fn card<Content: View>(content: Content) -> Card<Content> {
    Card::new(content)
}
