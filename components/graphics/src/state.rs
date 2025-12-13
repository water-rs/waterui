//! Canvas drawing state management.
//!
//! This module provides the state stack for HTML5 Canvas-style save/restore operations.

use waterui_color::Color;

// Internal imports for rendering (not exposed to users)
use vello::{kurbo, peniko};

use crate::gradient::{ConicGradient, LinearGradient, RadialGradient};
use crate::text::FontSpec;

// ============================================================================
// Fill and Stroke Styles
// ============================================================================

/// Fill style for shapes - can be a solid color or gradient.
#[derive(Clone)]
pub enum FillStyle {
    /// Solid color fill.
    Color(Color),
    /// Linear gradient fill.
    LinearGradient(LinearGradient),
    /// Radial gradient fill.
    RadialGradient(RadialGradient),
    /// Conic (sweep) gradient fill.
    ConicGradient(ConicGradient),
}

impl From<Color> for FillStyle {
    fn from(color: Color) -> Self {
        Self::Color(color)
    }
}

impl From<LinearGradient> for FillStyle {
    fn from(gradient: LinearGradient) -> Self {
        Self::LinearGradient(gradient)
    }
}

impl From<RadialGradient> for FillStyle {
    fn from(gradient: RadialGradient) -> Self {
        Self::RadialGradient(gradient)
    }
}

impl From<ConicGradient> for FillStyle {
    fn from(gradient: ConicGradient) -> Self {
        Self::ConicGradient(gradient)
    }
}

/// Stroke style for shapes - can be a solid color or gradient.
#[derive(Clone)]
pub enum StrokeStyle {
    /// Solid color stroke.
    Color(Color),
    /// Linear gradient stroke.
    LinearGradient(LinearGradient),
    /// Radial gradient stroke.
    RadialGradient(RadialGradient),
    /// Conic (sweep) gradient stroke.
    ConicGradient(ConicGradient),
}

impl From<Color> for StrokeStyle {
    fn from(color: Color) -> Self {
        Self::Color(color)
    }
}

impl From<LinearGradient> for StrokeStyle {
    fn from(gradient: LinearGradient) -> Self {
        Self::LinearGradient(gradient)
    }
}

impl From<RadialGradient> for StrokeStyle {
    fn from(gradient: RadialGradient) -> Self {
        Self::RadialGradient(gradient)
    }
}

impl From<ConicGradient> for StrokeStyle {
    fn from(gradient: ConicGradient) -> Self {
        Self::ConicGradient(gradient)
    }
}

// ============================================================================
// Line Styling
// ============================================================================

/// Line cap style (end of strokes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LineCap {
    /// Flat edge at end of line (default).
    #[default]
    Butt,
    /// Rounded end.
    Round,
    /// Square end extending beyond the endpoint.
    Square,
}

impl From<LineCap> for kurbo::Cap {
    fn from(cap: LineCap) -> Self {
        match cap {
            LineCap::Butt => Self::Butt,
            LineCap::Round => Self::Round,
            LineCap::Square => Self::Square,
        }
    }
}

/// Line join style (corners).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LineJoin {
    /// Miter join (sharp corner, default).
    #[default]
    Miter,
    /// Rounded corner.
    Round,
    /// Beveled corner (flattened).
    Bevel,
}

impl From<LineJoin> for kurbo::Join {
    fn from(join: LineJoin) -> Self {
        match join {
            LineJoin::Miter => Self::Miter,
            LineJoin::Round => Self::Round,
            LineJoin::Bevel => Self::Bevel,
        }
    }
}

// ============================================================================
// Text Styling (Phase 5)
// ============================================================================

/// Text alignment (future - Phase 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextAlign {
    /// Align to start edge (default).
    #[default]
    Start,
    /// Align to end edge.
    End,
    /// Align to left edge.
    Left,
    /// Align to right edge.
    Right,
    /// Center alignment.
    Center,
}

/// Text baseline (future - Phase 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextBaseline {
    /// Top of em square.
    Top,
    /// Hanging baseline.
    Hanging,
    /// Middle of em square.
    Middle,
    /// Alphabetic baseline (default).
    #[default]
    Alphabetic,
    /// Ideographic baseline.
    Ideographic,
    /// Bottom of em square.
    Bottom,
}

// ============================================================================
// Drawing State
// ============================================================================

/// Complete drawing state that can be saved and restored.
///
/// This represents the HTML5 Canvas drawing state, including transforms,
/// styles, and text settings.
#[derive(Clone)]
pub struct DrawingState {
    // Transform
    pub(crate) transform: kurbo::Affine,

    // Fill and Stroke
    pub(crate) fill_style: FillStyle,
    pub(crate) stroke_style: StrokeStyle,

    // Line styling
    pub(crate) line_width: f32,
    pub(crate) line_cap: LineCap,
    pub(crate) line_join: LineJoin,
    pub(crate) miter_limit: f32,
    pub(crate) line_dash: Vec<f32>,
    pub(crate) line_dash_offset: f32,

    // Global compositing
    pub(crate) global_alpha: f32,
    pub(crate) blend_mode: peniko::BlendMode,

    // Text styling (Phase 5)
    pub(crate) text_align: TextAlign,
    pub(crate) text_baseline: TextBaseline,
    pub(crate) font: FontSpec,

    // Shadow (Phase 7)
    pub(crate) shadow_blur: f32,
    pub(crate) shadow_color: Color,
    pub(crate) shadow_offset_x: f32,
    pub(crate) shadow_offset_y: f32,

    // Fill rule (Phase 7)
    pub(crate) fill_rule: peniko::Fill,
}

impl Default for DrawingState {
    fn default() -> Self {
        Self {
            transform: kurbo::Affine::IDENTITY,
            fill_style: FillStyle::Color(Color::srgb(0, 0, 0)), // Black
            stroke_style: StrokeStyle::Color(Color::srgb(0, 0, 0)), // Black
            line_width: 1.0,
            line_cap: LineCap::default(),
            line_join: LineJoin::default(),
            miter_limit: 10.0,
            line_dash: Vec::new(),
            line_dash_offset: 0.0,
            global_alpha: 1.0,
            blend_mode: peniko::BlendMode::default(),
            text_align: TextAlign::default(),
            text_baseline: TextBaseline::default(),
            font: FontSpec::default(),
            shadow_blur: 0.0,
            shadow_color: Color::srgb(0, 0, 0).with_opacity(0.0), // Transparent black
            shadow_offset_x: 0.0,
            shadow_offset_y: 0.0,
            fill_rule: peniko::Fill::NonZero, // Default fill rule
        }
    }
}

impl DrawingState {
    /// Creates a new drawing state with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a kurbo::Stroke from the current line styling state.
    #[must_use]
    pub(crate) fn build_stroke(&self) -> kurbo::Stroke {
        let mut stroke = kurbo::Stroke::new(f64::from(self.line_width))
            .with_caps(self.line_cap.into())
            .with_join(self.line_join.into())
            .with_miter_limit(f64::from(self.miter_limit));

        if !self.line_dash.is_empty() {
            let dashes: Vec<f64> = self.line_dash.iter().map(|&d| f64::from(d)).collect();
            stroke = stroke.with_dashes(f64::from(self.line_dash_offset), dashes);
        }

        stroke
    }
}
