//! The code-block look of the theme contract: what a theme returns for a
//! syntax-highlighted code block ([`CodeLook`]).
//!
//! A code block draws a [`Surface`] behind a header row (language caption +
//! copy action) and monospaced syntax-highlighted source text using
//! [`CodeMetrics`] padding and spacing.

use cherenkov_record::{Paint, WorkingColor};
use waterui_core::Environment;
use waterui_graphics::color::{
    Color,
    token::{Accent, Border as BorderToken, MutedForeground, SurfaceVariant},
};
use waterui_text::font::{Body, Caption, Font};

use super::{Border, CornerShape, Driven, Surface};

/// Default code-block corner radius in logical points.
const DEFAULT_CORNER_RADIUS: f32 = 12.0;
/// Default inner padding on all four edges of a code block in logical points.
const DEFAULT_PADDING: f32 = 14.0;
/// Default vertical gap between the header row and code body in logical points.
const DEFAULT_HEADER_SPACING: f32 = 8.0;
/// Default outline stroke width in logical points.
const DEFAULT_BORDER_WIDTH: f32 = 1.0;

/// Layout metrics for a code block container in logical points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CodeMetrics {
    /// Horizontal inner padding on leading and trailing edges.
    pub horizontal_padding: f32,
    /// Vertical inner padding on top and bottom edges.
    pub vertical_padding: f32,
    /// Vertical gap between the header row and the code body.
    pub header_spacing: f32,
}

impl CodeMetrics {
    /// Creates code-block layout metrics with uniform inner `padding` and
    /// vertical `header_spacing`.
    #[must_use]
    pub const fn uniform(padding: f32, header_spacing: f32) -> Self {
        Self {
            horizontal_padding: padding,
            vertical_padding: padding,
            header_spacing,
        }
    }
}

/// Visual look and layout metrics of a syntax-highlighted code block.
#[derive(Debug, Clone)]
pub struct CodeLook {
    surface: Surface,
    metrics: CodeMetrics,
    header_font: Font,
    header_color: Color,
    copy_color: Color,
    copied_color: Color,
    body_font: Font,
}

impl CodeLook {
    /// Creates a code-block look from its container `surface` and layout
    /// `metrics`, using semantic defaults (`Caption` + `MutedForeground`
    /// header, `Accent` copy confirmation tint, and monospaced `Body` source).
    #[must_use]
    pub fn new(surface: Surface, metrics: CodeMetrics) -> Self {
        Self {
            surface,
            metrics,
            header_font: Font::from(Caption),
            header_color: Color::new(MutedForeground),
            copy_color: Color::new(Accent),
            copied_color: Color::new(Accent),
            body_font: Font::from(Body).monospaced(true),
        }
    }

    /// Sets the font applied to the header language label.
    #[must_use]
    pub fn header_font(mut self, font: impl Into<Font>) -> Self {
        self.header_font = font.into();
        self
    }

    /// Sets the foreground color applied to the header language label.
    #[must_use]
    pub fn header_color(mut self, color: impl Into<Color>) -> Self {
        self.header_color = color.into();
        self
    }

    /// Sets the foreground tint applied to the header copy button at rest.
    #[must_use]
    pub fn copy_color(mut self, color: impl Into<Color>) -> Self {
        self.copy_color = color.into();
        self
    }

    /// Sets the foreground tint applied to the header copy button while
    /// confirming copy feedback.
    #[must_use]
    pub fn copied_color(mut self, color: impl Into<Color>) -> Self {
        self.copied_color = color.into();
        self
    }

    /// Sets the font applied to the code block body.
    #[must_use]
    pub fn body_font(mut self, font: impl Into<Font>) -> Self {
        self.body_font = font.into();
        self
    }

    /// The container surface: shape, fill or backdrop, border, and elevation.
    #[must_use]
    pub const fn surface(&self) -> &Surface {
        &self.surface
    }

    /// Inner padding and header-to-body spacing.
    #[must_use]
    pub const fn metrics(&self) -> CodeMetrics {
        self.metrics
    }

    /// Font for the header language caption.
    #[must_use]
    pub const fn header_font_ref(&self) -> &Font {
        &self.header_font
    }

    /// Foreground color for the header language caption.
    #[must_use]
    pub const fn header_color_ref(&self) -> &Color {
        &self.header_color
    }

    /// Foreground tint for the header copy action at rest.
    #[must_use]
    pub const fn copy_color_ref(&self) -> &Color {
        &self.copy_color
    }

    /// Foreground tint for the header copy action while confirming a copy.
    #[must_use]
    pub const fn copied_color_ref(&self) -> &Color {
        &self.copied_color
    }

    /// Font for the syntax-highlighted code body.
    #[must_use]
    pub const fn body_font_ref(&self) -> &Font {
        &self.body_font
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

/// Baseline semantic code-block look built from theme tokens, reusable by test
/// themes implementing [`WidgetTheme::code`](super::WidgetTheme::code).
#[must_use]
pub fn default_code_look(env: &Environment) -> CodeLook {
    let shape = CornerShape::radius(DEFAULT_CORNER_RADIUS).continuous();
    let surface = Surface::new(shape, token_paint(&Color::new(SurfaceVariant), env)).border(
        token_border(DEFAULT_BORDER_WIDTH, &Color::new(BorderToken), env),
    );
    CodeLook::new(
        surface,
        CodeMetrics::uniform(DEFAULT_PADDING, DEFAULT_HEADER_SPACING),
    )
}
