//! Text rendering support for Canvas.
//!
//! This module provides text layout and rendering capabilities using Parley
//! for text layout and Vello for glyph rendering.

// Internal imports for text rendering
use parley::{FontContext, LayoutContext};
use vello::peniko;

/// Text rendering context that manages font and layout.
///
/// This is used internally by the Canvas to render text.
pub(crate) struct TextContext {
    font_cx: FontContext,
    layout_cx: LayoutContext<peniko::Brush>,
}

impl TextContext {
    /// Creates a new text context.
    pub(crate) fn new() -> Self {
        Self {
            font_cx: FontContext::default(),
            layout_cx: LayoutContext::new(),
        }
    }

    /// Returns a reference to the font context.
    pub(crate) fn font_context(&mut self) -> &mut FontContext {
        &mut self.font_cx
    }

    /// Returns a reference to the layout context.
    pub(crate) fn layout_context(&mut self) -> &mut LayoutContext<peniko::Brush> {
        &mut self.layout_cx
    }
}

/// Text metrics information.
///
/// Provides measurements for rendered text.
#[derive(Debug, Clone, Copy)]
pub struct TextMetrics {
    /// Width of the text in pixels.
    pub width: f32,
    /// Height of the text in pixels.
    pub height: f32,
}

impl TextMetrics {
    /// Creates new text metrics.
    #[must_use]
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

/// Font style for text rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontStyle {
    /// Normal font style.
    #[default]
    Normal,
    /// Italic font style.
    Italic,
    /// Oblique font style.
    Oblique,
}

/// Font weight for text rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontWeight {
    /// Thin weight (100).
    Thin,
    /// Extra light weight (200).
    ExtraLight,
    /// Light weight (300).
    Light,
    /// Normal weight (400).
    #[default]
    Normal,
    /// Medium weight (500).
    Medium,
    /// Semi-bold weight (600).
    SemiBold,
    /// Bold weight (700).
    Bold,
    /// Extra bold weight (800).
    ExtraBold,
    /// Black weight (900).
    Black,
}

impl FontWeight {
    /// Returns the numeric value of the font weight.
    #[must_use]
    pub const fn value(self) -> u16 {
        match self {
            Self::Thin => 100,
            Self::ExtraLight => 200,
            Self::Light => 300,
            Self::Normal => 400,
            Self::Medium => 500,
            Self::SemiBold => 600,
            Self::Bold => 700,
            Self::ExtraBold => 800,
            Self::Black => 900,
        }
    }
}

/// Font specification for text rendering.
#[derive(Debug, Clone)]
pub struct FontSpec {
    /// Font family name.
    pub family: String,
    /// Font size in pixels.
    pub size: f32,
    /// Font weight.
    pub weight: FontWeight,
    /// Font style.
    pub style: FontStyle,
}

impl FontSpec {
    /// Creates a new font specification.
    #[must_use]
    pub fn new(family: impl Into<String>, size: f32) -> Self {
        Self {
            family: family.into(),
            size,
            weight: FontWeight::default(),
            style: FontStyle::default(),
        }
    }

    /// Sets the font weight.
    #[must_use]
    pub fn with_weight(mut self, weight: FontWeight) -> Self {
        self.weight = weight;
        self
    }

    /// Sets the font style.
    #[must_use]
    pub fn with_style(mut self, style: FontStyle) -> Self {
        self.style = style;
        self
    }
}

impl Default for FontSpec {
    fn default() -> Self {
        Self {
            family: "sans-serif".to_string(),
            size: 16.0,
            weight: FontWeight::default(),
            style: FontStyle::default(),
        }
    }
}
