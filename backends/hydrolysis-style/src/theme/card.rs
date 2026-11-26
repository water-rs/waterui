//! The card look of the theme contract: what a theme returns for a
//! [`CardStyle`] ([`CardLook`]).
//!
//! A card draws a [`Surface`] behind optional title, subtitle, and body
//! content stacked vertically with [`CardLayout`] padding and spacing.

use cherenkov_record::{Paint, Shadow, WorkingColor};
use kurbo::Vec2;
use waterui_controls::card::CardStyle;
use waterui_core::Environment;
use waterui_graphics::color::{
    Color,
    token::{Border as BorderToken, MutedForeground, Surface as SurfaceToken, SurfaceVariant},
};
use waterui_text::font::{Font, Subheadline, Title};

use super::{Border, CornerShape, Driven, Elevation, Surface};

/// Default card corner radius in logical points (matching Material 3 / Apple card chrome).
const DEFAULT_CORNER_RADIUS: f32 = 12.0;
/// Default horizontal and vertical inner card padding in logical points.
const DEFAULT_PADDING: f32 = 16.0;
/// Default vertical gap between title, subtitle, and content in logical points.
const DEFAULT_CONTENT_SPACING: f32 = 6.0;
/// Default outline stroke width in logical points.
const DEFAULT_BORDER_WIDTH: f32 = 1.0;

/// Layout metrics for a card container in logical points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardLayout {
    /// Horizontal inner padding on leading and trailing edges.
    pub horizontal_padding: f32,
    /// Vertical inner padding on top and bottom edges.
    pub vertical_padding: f32,
    /// Vertical gap between title, subtitle, and body content.
    pub content_spacing: f32,
}

impl CardLayout {
    /// Creates a card layout with uniform inner `padding` and vertical `content_spacing`.
    #[must_use]
    pub const fn uniform(padding: f32, content_spacing: f32) -> Self {
        Self {
            horizontal_padding: padding,
            vertical_padding: padding,
            content_spacing,
        }
    }
}

/// Visual look and layout metrics of a card container.
#[derive(Debug, Clone)]
pub struct CardLook {
    surface: Surface,
    layout: CardLayout,
    title_font: Font,
    subtitle_font: Font,
    subtitle_color: Color,
}

impl CardLook {
    /// Creates a card look from its container `surface` and `layout` metrics,
    /// using semantic default typography (`Title`, `Subheadline`, `MutedForeground`).
    #[must_use]
    pub fn new(surface: Surface, layout: CardLayout) -> Self {
        Self {
            surface,
            layout,
            title_font: Font::from(Title),
            subtitle_font: Font::from(Subheadline),
            subtitle_color: Color::new(MutedForeground),
        }
    }

    /// Sets the default font applied to an unstyled card title.
    #[must_use]
    pub fn title_font(mut self, font: impl Into<Font>) -> Self {
        self.title_font = font.into();
        self
    }

    /// Sets the default font applied to an unstyled card subtitle.
    #[must_use]
    pub fn subtitle_font(mut self, font: impl Into<Font>) -> Self {
        self.subtitle_font = font.into();
        self
    }

    /// Sets the default foreground color applied to an unstyled card subtitle.
    #[must_use]
    pub fn subtitle_color(mut self, color: impl Into<Color>) -> Self {
        self.subtitle_color = color.into();
        self
    }

    /// The card's container surface: shape, fill or backdrop, border, and elevation.
    #[must_use]
    pub const fn surface(&self) -> &Surface {
        &self.surface
    }

    /// Inner padding and inter-element spacing.
    #[must_use]
    pub const fn layout(&self) -> CardLayout {
        self.layout
    }

    /// Default font for the card title.
    #[must_use]
    pub const fn title_font_ref(&self) -> &Font {
        &self.title_font
    }

    /// Default font for the card subtitle.
    #[must_use]
    pub const fn subtitle_font_ref(&self) -> &Font {
        &self.subtitle_font
    }

    /// Default foreground color for the card subtitle.
    #[must_use]
    pub const fn subtitle_color_ref(&self) -> &Color {
        &self.subtitle_color
    }
}

fn token_paint(color: &Color, env: &Environment) -> Driven<Paint> {
    Driven::follow(color.resolve(env), |resolved: &WorkingColor, _| {
        Paint::from(*resolved)
    })
}

fn token_border(width: f32, color: &Color, env: &Environment) -> Driven<Border> {
    Driven::follow(color.resolve(env), move |resolved: &WorkingColor, _| {
        Border::new(width, Paint::from(*resolved))
    })
}

/// Default framework card look built from semantic theme tokens (`Surface`,
/// `SurfaceVariant`, `Border`), used until a style package overrides
/// [`WidgetTheme::card`](super::WidgetTheme::card).
#[must_use]
pub fn default_card_look(style: CardStyle, env: &Environment) -> CardLook {
    let shape = CornerShape::radius(DEFAULT_CORNER_RADIUS).continuous();
    let surface =
        match style {
            CardStyle::Automatic | CardStyle::Outlined => {
                Surface::new(shape, token_paint(&Color::new(SurfaceToken), env)).border(
                    token_border(DEFAULT_BORDER_WIDTH, &Color::new(BorderToken), env),
                )
            }
            CardStyle::Filled => Surface::new(shape, token_paint(&Color::new(SurfaceVariant), env)),
            CardStyle::Elevated => Surface::new(shape, token_paint(&Color::new(SurfaceToken), env))
                .elevation(Elevation::new(
                    Shadow::new(4.0, WorkingColor::new([0.0, 0.0, 0.0, 0.14]))
                        .offset(Vec2::new(0.0, 2.0)),
                    Shadow::new(1.5, WorkingColor::new([0.0, 0.0, 0.0, 0.08])),
                )),
            _ => Surface::new(shape, token_paint(&Color::new(SurfaceToken), env)).border(
                token_border(DEFAULT_BORDER_WIDTH, &Color::new(BorderToken), env),
            ),
        };
    CardLook::new(
        surface,
        CardLayout::uniform(DEFAULT_PADDING, DEFAULT_CONTENT_SPACING),
    )
}
