//! Math formula rendering for WaterUI using Vello.
//!
//! The crate provides a `Math` view that accepts LaTeX, MathML, or Typst input,
//! parses a practical subset, lays out the expression, and renders vector output
//! through Vello.

#![allow(clippy::multiple_crate_versions)]

extern crate alloc;

use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use kurbo::{Affine, ParamCurve};
use parley::{Alignment, AlignmentOptions, FontContext, LayoutContext, StyleProperty};
use peniko::{Brush, Color as PenikoColor, Fill};
use ttf_parser::{Face, GlyphId, OutlineBuilder};
use waterui_core::{Environment, Signal, View};
use waterui_graphics::color::{Color, ForegroundColor, ResolvedColor, Srgb};
use waterui_graphics::{Scene2D, SceneContent, SceneView};
use waterui_layout::frame::Frame;
use waterui_str::Str;

/// Input syntax accepted by [`Math`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MathInputFormat {
    /// LaTeX math subset.
    #[default]
    Latex,
    /// MathML Core subset.
    MathMl,
    /// Typst math subset.
    Typst,
}

/// Display mode for formula layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MathDisplayMode {
    /// Inline formula intended to sit inside text flow.
    #[default]
    Inline,
    /// Block formula with larger outer padding.
    Block,
}

/// GPU-rendered math formula view.
#[derive(Debug, Clone)]
pub struct Math {
    source: Str,
    format: MathInputFormat,
    display: MathDisplayMode,
    font_size: f32,
    font_stack: Str,
    color: Option<Color>,
}

const DEFAULT_MATH_FONT_STACK: &str =
    "STIX Two Math, Noto Sans Math, Latin Modern Math, Cambria Math, serif";
const RADICAL_GLYPH: &str = "√";

impl Math {
    /// Creates a LaTeX formula view.
    #[must_use]
    pub fn new(latex: impl Into<Str>) -> Self {
        Self::latex(latex)
    }

    /// Creates a LaTeX formula view.
    #[must_use]
    pub fn latex(source: impl Into<Str>) -> Self {
        Self {
            source: source.into(),
            format: MathInputFormat::Latex,
            display: MathDisplayMode::Inline,
            font_size: 18.0,
            font_stack: Str::from(DEFAULT_MATH_FONT_STACK),
            color: None,
        }
    }

    /// Creates a MathML formula view.
    #[must_use]
    pub fn mathml(source: impl Into<Str>) -> Self {
        Self {
            source: source.into(),
            format: MathInputFormat::MathMl,
            ..Self::latex(Str::from_static(""))
        }
    }

    /// Creates a Typst formula view.
    #[must_use]
    pub fn typst(source: impl Into<Str>) -> Self {
        Self {
            source: source.into(),
            format: MathInputFormat::Typst,
            ..Self::latex(Str::from_static(""))
        }
    }

    /// Sets display mode to block.
    #[must_use]
    pub fn block(mut self) -> Self {
        self.display = MathDisplayMode::Block;
        self
    }

    /// Sets display mode to inline.
    #[must_use]
    pub fn inline(mut self) -> Self {
        self.display = MathDisplayMode::Inline;
        self
    }

    /// Sets the rendering font size.
    ///
    /// Panics if the value is not finite and positive.
    #[must_use]
    pub fn font_size(mut self, size: f32) -> Self {
        assert!(
            size.is_finite() && size > 0.0,
            "Math font size must be positive and finite"
        );
        self.font_size = size;
        self
    }

    /// Sets the primary font family used for glyph shaping.
    #[must_use]
    pub fn font_family(mut self, family: impl Into<Str>) -> Self {
        let family = family.into();
        let family = family.as_str().trim().to_string();
        self.font_stack = if family.is_empty() {
            Str::from(DEFAULT_MATH_FONT_STACK)
        } else if family.contains(',') {
            Str::from(family)
        } else {
            Str::from(format!("{family}, {DEFAULT_MATH_FONT_STACK}"))
        };
        self
    }

    /// Sets full CSS-like font stack used for glyph shaping.
    #[must_use]
    pub fn font_stack(mut self, stack: impl Into<Str>) -> Self {
        let stack = stack.into();
        assert!(
            !stack.as_str().trim().is_empty(),
            "Math font stack must not be empty"
        );
        self.font_stack = stack;
        self
    }

    /// Sets explicit formula color.
    #[must_use]
    pub fn color(mut self, color: impl Into<Color>) -> Self {
        self.color = Some(color.into());
        self
    }
}

/// Convenience constructor for creating LaTeX formulas inline.
#[must_use]
pub fn math(source: impl Into<Str>) -> Math {
    Math::latex(source)
}

impl View for Math {
    fn body(self, env: &Environment) -> impl View {
        let expression =
            parse_math_source(self.format, self.source.as_str()).unwrap_or_else(|err| {
                panic!(
                    "Failed to parse math expression in {:?} mode: {err}. Source: {}",
                    self.format, self.source
                )
            });

        let mut text_engine = MathTextEngine::new(self.font_stack.as_str());
        let mut layouter = FormulaLayouter::new(&mut text_engine);
        let layout = layouter
            .layout(&expression, self.font_size)
            .unwrap_or_else(|err| panic!("Failed to layout math expression: {err}"));

        let color = resolve_math_color(env, self.color.as_ref());
        let padding = match self.display {
            MathDisplayMode::Inline => self.font_size * 0.14,
            MathDisplayMode::Block => self.font_size * 0.36,
        };

        let content = MathSceneContent::new(layout, color, self.font_stack.as_str(), padding);
        let (width, height) = content.intrinsic_size();

        Frame::new(SceneView::new(content))
            .width(width)
            .height(height)
    }
}

struct MathSceneContent {
    base_scene: vello::Scene,
    formula_width: f32,
    formula_height: f32,
    padding: f32,
}

impl MathSceneContent {
    fn new(layout: FormulaLayout, color: PenikoColor, font_stack: &str, padding: f32) -> Self {
        let formula_height = layout.height();
        let mut base_scene = vello::Scene::new();
        let brush: Brush = color.into();
        let baseline_y = padding + layout.ascent;
        let mut text_engine = MathTextEngine::new(font_stack);

        for command in layout.commands {
            match command {
                DrawCommand::Text {
                    text,
                    x,
                    baseline,
                    font_size,
                } => {
                    text_engine.draw_text(
                        &mut base_scene,
                        &brush,
                        &text,
                        font_size,
                        padding + x,
                        baseline_y + baseline,
                    );
                }
                DrawCommand::Rect {
                    x,
                    y,
                    width,
                    height,
                } => {
                    let shape = kurbo::Rect::new(
                        f64::from(padding + x),
                        f64::from(baseline_y + y),
                        f64::from(padding + x + width.max(0.0)),
                        f64::from(baseline_y + y + height.max(0.0)),
                    );
                    base_scene.fill(Fill::NonZero, Affine::IDENTITY, &brush, None, &shape);
                }
                DrawCommand::Path { path, x, y } => {
                    let transform =
                        Affine::translate((f64::from(padding + x), f64::from(baseline_y + y)));
                    base_scene.fill(Fill::NonZero, transform, &brush, None, &path);
                }
            }
        }

        Self {
            base_scene,
            formula_width: layout.width,
            formula_height,
            padding,
        }
    }

    fn intrinsic_size(&self) -> (f32, f32) {
        (
            (self.formula_width + self.padding * 2.0).max(1.0),
            (self.formula_height + self.padding * 2.0).max(1.0),
        )
    }
}

impl SceneContent for MathSceneContent {
    fn build_scene(&mut self, scene: &mut dyn Scene2D, width: f32, height: f32) -> bool {
        if !(width.is_finite() && height.is_finite()) || width <= 0.0 || height <= 0.0 {
            return false;
        }

        let (intrinsic_width, intrinsic_height) = self.intrinsic_size();
        let scale_x = width / intrinsic_width;
        let scale_y = height / intrinsic_height;
        let scale = scale_x.min(scale_y).max(0.0);
        let offset_x = (width - intrinsic_width * scale) * 0.5;
        let offset_y = (height - intrinsic_height * scale) * 0.5;

        let transform = Affine::translate((f64::from(offset_x), f64::from(offset_y)))
            * Affine::scale(f64::from(scale));
        scene.append_vello_scene(&self.base_scene, Some(transform));
        false
    }
}

#[derive(Debug, Clone)]
struct FormulaLayout {
    width: f32,
    ascent: f32,
    descent: f32,
    commands: Vec<DrawCommand>,
}

impl FormulaLayout {
    fn empty(font_size: f32) -> Self {
        Self {
            width: 0.0,
            ascent: font_size * 0.72,
            descent: font_size * 0.28,
            commands: Vec::new(),
        }
    }

    fn height(&self) -> f32 {
        self.ascent + self.descent
    }
}

#[derive(Debug, Clone)]
enum DrawCommand {
    Text {
        text: String,
        x: f32,
        baseline: f32,
        font_size: f32,
    },
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
    Path {
        path: kurbo::BezPath,
        x: f32,
        y: f32,
    },
}

impl DrawCommand {
    fn translated(self, dx: f32, dy: f32) -> Self {
        match self {
            Self::Text {
                text,
                x,
                baseline,
                font_size,
            } => Self::Text {
                text,
                x: x + dx,
                baseline: baseline + dy,
                font_size,
            },
            Self::Rect {
                x,
                y,
                width,
                height,
            } => Self::Rect {
                x: x + dx,
                y: y + dy,
                width,
                height,
            },
            Self::Path { path, x, y } => Self::Path {
                path,
                x: x + dx,
                y: y + dy,
            },
        }
    }
}

#[derive(Debug, Clone)]
enum MathNode {
    Row(Vec<MathNode>),
    Symbol(String),
    Fraction {
        numerator: Box<MathNode>,
        denominator: Box<MathNode>,
    },
    SupSub {
        base: Box<MathNode>,
        superscript: Option<Box<MathNode>>,
        subscript: Option<Box<MathNode>>,
    },
    Sqrt {
        radicand: Box<MathNode>,
    },
}

#[derive(Debug, Clone)]
struct MathError {
    message: String,
}

impl MathError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for MathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl core::error::Error for MathError {}

#[derive(Debug, Clone, Copy)]
struct TextMetrics {
    width: f32,
    ascent: f32,
    descent: f32,
}

struct MathTextEngine {
    font_context: FontContext,
    layout_context: LayoutContext,
    font_stack: String,
}

impl MathTextEngine {
    fn new(font_stack: &str) -> Self {
        Self {
            font_context: FontContext::new(),
            layout_context: LayoutContext::new(),
            font_stack: resolve_math_font_stack(font_stack),
        }
    }

    fn build_layout(&mut self, text: &str, font_size: f32) -> parley::Layout<[u8; 4]> {
        let mut builder =
            self.layout_context
                .ranged_builder(&mut self.font_context, text, 1.0, true);
        builder.push_default(StyleProperty::Brush([255, 255, 255, 255]));
        builder.push_default(StyleProperty::FontSize(font_size));
        if !self.font_stack.trim().is_empty() {
            builder.push_default(StyleProperty::FontStack(parley::FontStack::Source(
                Cow::Owned(self.font_stack.clone()),
            )));
        }

        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout.align(None, Alignment::Start, AlignmentOptions::default());
        layout
    }

    fn measure_text(&mut self, text: &str, font_size: f32) -> TextMetrics {
        if text.is_empty() {
            return TextMetrics {
                width: 0.0,
                ascent: font_size * 0.72,
                descent: font_size * 0.28,
            };
        }

        let layout = self.build_layout(text, font_size);
        let line = layout.lines().next().unwrap_or_else(|| {
            panic!(
                "Math text shaping produced no lines for non-empty text '{}', size {}",
                text, font_size
            )
        });
        let metrics = line.metrics();
        TextMetrics {
            width: layout.width(),
            ascent: metrics.ascent,
            descent: metrics.descent,
        }
    }

    fn resolve_font_data(
        &mut self,
        text: &str,
        font_size: f32,
    ) -> Result<peniko::FontData, MathError> {
        if text.is_empty() {
            return Err(MathError::new("cannot resolve font for empty text"));
        }

        let layout = self.build_layout(text, font_size);
        for line in layout.lines() {
            for item in line.items() {
                if let parley::PositionedLayoutItem::GlyphRun(glyph_run) = item {
                    return Ok(glyph_run.run().font().clone());
                }
            }
        }

        Err(MathError::new(format!(
            "unable to resolve shaped font for text '{}'",
            text
        )))
    }

    fn draw_text(
        &mut self,
        scene: &mut vello::Scene,
        brush: &Brush,
        text: &str,
        font_size: f32,
        x: f32,
        baseline: f32,
    ) {
        if text.is_empty() {
            return;
        }

        let layout = self.build_layout(text, font_size);
        let transform = Affine::translate((f64::from(x), f64::from(baseline)));

        for line in layout.lines() {
            for item in line.items() {
                if let parley::PositionedLayoutItem::GlyphRun(glyph_run) = item {
                    let run = glyph_run.run();
                    let normalized_coords: Vec<vello::NormalizedCoord> =
                        run.normalized_coords().to_vec();
                    let mut run_x = glyph_run.offset();
                    let glyphs = glyph_run.glyphs().map(move |glyph| {
                        let x = run_x + glyph.x;
                        // `baseline` is already encoded in the outer transform, so glyph y
                        // should stay in run-local coordinates to avoid double baseline offsets.
                        let y = -glyph.y;
                        run_x += glyph.advance;
                        vello::Glyph { id: glyph.id, x, y }
                    });

                    scene
                        .draw_glyphs(run.font())
                        .brush(brush)
                        .transform(transform)
                        .font_size(run.font_size())
                        .normalized_coords(&normalized_coords)
                        .draw(Fill::NonZero, glyphs);
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
struct RadicalOutline {
    path: kurbo::BezPath,
    height: f32,
}

#[derive(Debug, Clone)]
struct AssemblyPart {
    glyph: GlyphOutline,
    full_advance_du: f32,
    start_connector_du: f32,
    end_connector_du: f32,
    extender: bool,
}

#[derive(Debug, Clone)]
struct GlyphOutline {
    path: kurbo::BezPath,
    x_min: f32,
    y_min: f32,
    x_max: f32,
    y_max: f32,
}

#[derive(Default)]
struct KurboOutlineBuilder {
    path: kurbo::BezPath,
}

impl OutlineBuilder for KurboOutlineBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path
            .move_to(kurbo::Point::new(f64::from(x), f64::from(-y)));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.path
            .line_to(kurbo::Point::new(f64::from(x), f64::from(-y)));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.path.quad_to(
            kurbo::Point::new(f64::from(x1), f64::from(-y1)),
            kurbo::Point::new(f64::from(x), f64::from(-y)),
        );
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.path.curve_to(
            kurbo::Point::new(f64::from(x1), f64::from(-y1)),
            kurbo::Point::new(f64::from(x2), f64::from(-y2)),
            kurbo::Point::new(f64::from(x), f64::from(-y)),
        );
    }

    fn close(&mut self) {
        self.path.close_path();
    }
}

fn parse_face_from_font(font_data: &peniko::FontData) -> Result<Face<'_>, MathError> {
    Face::parse(font_data.data.data(), font_data.index).map_err(|err| {
        MathError::new(format!(
            "failed to parse font face (index {}): {err:?}",
            font_data.index
        ))
    })
}

fn load_normalized_glyph_path(
    face: &Face<'_>,
    glyph_id: GlyphId,
    scale: f32,
) -> Result<GlyphOutline, MathError> {
    let bbox = face
        .glyph_bounding_box(glyph_id)
        .ok_or_else(|| MathError::new(format!("glyph {glyph_id:?} has no bounding box")))?;

    let mut builder = KurboOutlineBuilder::default();
    face.outline_glyph(glyph_id, &mut builder)
        .ok_or_else(|| MathError::new(format!("glyph {glyph_id:?} has no vector outline")))?;

    let mut path = builder.path;
    path.apply_affine(Affine::scale(f64::from(scale)));

    let x_min = f32::from(bbox.x_min) * scale;
    let x_max = f32::from(bbox.x_max) * scale;
    let y_min = -f32::from(bbox.y_max) * scale;
    let y_max = -f32::from(bbox.y_min) * scale;

    Ok(GlyphOutline {
        path,
        x_min,
        y_min,
        x_max,
        y_max,
    })
}

fn path_x_range_in_y_band(
    path: &kurbo::BezPath,
    band_min_y: f32,
    band_max_y: f32,
) -> Option<(f32, f32)> {
    if !(band_min_y.is_finite() && band_max_y.is_finite()) {
        return None;
    }
    let (min_y, max_y) = if band_min_y <= band_max_y {
        (band_min_y, band_max_y)
    } else {
        (band_max_y, band_min_y)
    };
    const SAMPLES_PER_SEGMENT: usize = 96;
    let mut leftmost_x = f32::INFINITY;
    let mut rightmost_x = f32::NEG_INFINITY;

    for segment in path.segments() {
        for step in 0..=SAMPLES_PER_SEGMENT {
            let t = step as f64 / SAMPLES_PER_SEGMENT as f64;
            let point = segment.eval(t);
            let y = point.y as f32;
            if y >= min_y && y <= max_y {
                leftmost_x = leftmost_x.min(point.x as f32);
                rightmost_x = rightmost_x.max(point.x as f32);
            }
        }
    }

    (leftmost_x.is_finite() && rightmost_x.is_finite()).then_some((leftmost_x, rightmost_x))
}

fn path_rightmost_x_in_y_band(
    path: &kurbo::BezPath,
    band_min_y: f32,
    band_max_y: f32,
) -> Option<f32> {
    path_x_range_in_y_band(path, band_min_y, band_max_y).map(|(_, rightmost_x)| rightmost_x)
}

fn path_y_range_in_x_band(
    path: &kurbo::BezPath,
    band_min_x: f32,
    band_max_x: f32,
) -> Option<(f32, f32)> {
    if !(band_min_x.is_finite() && band_max_x.is_finite()) {
        return None;
    }
    let (min_x, max_x) = if band_min_x <= band_max_x {
        (band_min_x, band_max_x)
    } else {
        (band_max_x, band_min_x)
    };
    const SAMPLES_PER_SEGMENT: usize = 96;
    let mut top_y = f32::INFINITY;
    let mut bottom_y = f32::NEG_INFINITY;

    for segment in path.segments() {
        for step in 0..=SAMPLES_PER_SEGMENT {
            let t = step as f64 / SAMPLES_PER_SEGMENT as f64;
            let point = segment.eval(t);
            let x = point.x as f32;
            if x >= min_x && x <= max_x {
                let y = point.y as f32;
                top_y = top_y.min(y);
                bottom_y = bottom_y.max(y);
            }
        }
    }

    (top_y.is_finite() && bottom_y.is_finite()).then_some((top_y, bottom_y))
}

fn sample_radical_top_bar_band(
    path: &kurbo::BezPath,
    join_x: f32,
    stroke_hint: f32,
) -> Option<(f32, f32)> {
    let probe_width = (stroke_hint * 0.42).max(0.6);
    let mut best: Option<(f32, f32, f32)> = None;
    let offsets = [0.35_f32, 0.55, 0.75, 0.95, 1.15];

    for factor in offsets {
        let x1 = (join_x - stroke_hint * factor).max(0.0);
        let x0 = (x1 - probe_width).max(0.0);
        let Some((top, bottom)) = path_y_range_in_x_band(path, x0, x1) else {
            continue;
        };
        let thickness = bottom - top;
        if !(thickness.is_finite() && thickness > 0.0) {
            continue;
        }
        match best {
            Some((_, _, current)) if current <= thickness => {}
            _ => best = Some((top, bottom, thickness)),
        }
    }

    best.map(|(top, bottom, _)| (top, bottom))
}

fn compress_radical_height_if_oversized(
    radical: &mut RadicalOutline,
    desired_height: f32,
) -> Result<(), MathError> {
    if !(desired_height.is_finite() && desired_height > 0.0) {
        return Err(MathError::new("invalid desired radical height"));
    }
    if !(radical.height.is_finite() && radical.height > 0.0) {
        return Err(MathError::new("invalid radical outline height"));
    }

    let max_unscaled = desired_height * 1.08;
    if radical.height <= max_unscaled {
        return Ok(());
    }

    let target_height = desired_height * 1.04;
    let scale_y = (target_height / radical.height).clamp(0.84, 1.0);
    radical
        .path
        .apply_affine(Affine::new([1.0, 0.0, 0.0, f64::from(scale_y), 0.0, 0.0]));
    radical.height *= scale_y;

    Ok(())
}

fn connector_overlap_design_units(
    previous: &AssemblyPart,
    next: &AssemblyPart,
    min_connector_overlap: u16,
) -> f32 {
    f32::from(min_connector_overlap).min(previous.end_connector_du.min(next.start_connector_du))
}

fn assembly_advance_design_units(parts: &[AssemblyPart], min_connector_overlap: u16) -> f32 {
    if parts.is_empty() {
        return 0.0;
    }

    let mut total = parts[0].full_advance_du;
    for pair in parts.windows(2) {
        let overlap = connector_overlap_design_units(&pair[0], &pair[1], min_connector_overlap);
        total += pair[1].full_advance_du - overlap;
    }
    total
}

fn build_radical_outline_from_assembly(
    face: &Face<'_>,
    assembly: ttf_parser::math::GlyphAssembly<'_>,
    min_connector_overlap: u16,
    target_advance_du: f32,
    scale: f32,
) -> Result<RadicalOutline, MathError> {
    let mut base_parts = Vec::new();
    for part in assembly.parts {
        base_parts.push(AssemblyPart {
            glyph: load_normalized_glyph_path(face, part.glyph_id, scale)?,
            full_advance_du: f32::from(part.full_advance),
            start_connector_du: f32::from(part.start_connector_length),
            end_connector_du: f32::from(part.end_connector_length),
            extender: part.part_flags.extender(),
        });
    }

    if base_parts.is_empty() {
        return Err(MathError::new("MATH radical assembly has no parts"));
    }

    let extender_index = base_parts
        .iter()
        .position(|part| part.extender)
        .ok_or_else(|| MathError::new("MATH radical assembly is missing an extender part"))?;

    if base_parts
        .iter()
        .enumerate()
        .skip(extender_index + 1)
        .any(|(_, part)| part.extender)
    {
        return Err(MathError::new(
            "MATH radical assembly with multiple extender slots is not supported",
        ));
    }

    let mut sequence = base_parts.clone();
    let mut guard = 0_usize;
    while assembly_advance_design_units(&sequence, min_connector_overlap) < target_advance_du {
        sequence.insert(extender_index + 1, base_parts[extender_index].clone());
        guard += 1;
        if guard > 64 {
            return Err(MathError::new(
                "MATH radical assembly required too many extender insertions",
            ));
        }
    }

    let mut assembled = kurbo::BezPath::new();
    let mut y_offset_du = 0.0_f32;
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;

    for (index, part) in sequence.iter().enumerate() {
        let y_offset = y_offset_du * scale;

        let mut path = part.glyph.path.clone();
        path.apply_affine(Affine::translate((0.0, f64::from(y_offset))));
        assembled.extend(path.elements().iter().copied());

        min_x = min_x.min(part.glyph.x_min);
        max_x = max_x.max(part.glyph.x_max);
        min_y = min_y.min(part.glyph.y_min + y_offset);
        max_y = max_y.max(part.glyph.y_max + y_offset);

        if index + 1 < sequence.len() {
            let next = &sequence[index + 1];
            let overlap = connector_overlap_design_units(part, next, min_connector_overlap);
            y_offset_du += part.full_advance_du - overlap;
        } else {
            y_offset_du += part.full_advance_du;
        }
    }

    if !min_x.is_finite() || !min_y.is_finite() || !max_x.is_finite() || !max_y.is_finite() {
        return Err(MathError::new(
            "assembled radical outline has invalid bounds",
        ));
    }
    if !(max_x > min_x && max_y > min_y) {
        return Err(MathError::new(
            "assembled radical outline has non-positive bounds",
        ));
    }

    assembled.apply_affine(Affine::translate((f64::from(-min_x), f64::from(-min_y))));
    Ok(RadicalOutline {
        path: assembled,
        height: max_y - min_y,
    })
}

fn build_radical_outline_from_math(
    face: &Face<'_>,
    radical_glyph: GlyphId,
    target_advance: f32,
    scale: f32,
) -> Result<RadicalOutline, MathError> {
    if !(scale.is_finite() && scale > 0.0) {
        return Err(MathError::new("invalid font scale for radical layout"));
    }
    if !(target_advance.is_finite() && target_advance > 0.0) {
        return Err(MathError::new("invalid radical target advance"));
    }

    let math = face
        .tables()
        .math
        .ok_or_else(|| MathError::new("selected font has no OpenType MATH table"))?;
    let variants = math
        .variants
        .ok_or_else(|| MathError::new("selected font MATH table has no variants section"))?;
    let construction = variants
        .vertical_constructions
        .get(radical_glyph)
        .ok_or_else(|| MathError::new("selected font has no vertical radical construction"))?;

    let target_advance_du = target_advance / scale;
    let mut chosen_variant = None;
    for variant in construction.variants {
        chosen_variant = Some(variant);
        if f32::from(variant.advance_measurement) >= target_advance_du {
            break;
        }
    }

    if let Some(assembly) = construction.assembly {
        let variant_too_small = chosen_variant
            .map(|variant| f32::from(variant.advance_measurement) < target_advance_du)
            .unwrap_or(true);
        if variant_too_small {
            return build_radical_outline_from_assembly(
                face,
                assembly,
                variants.min_connector_overlap,
                target_advance_du,
                scale,
            );
        }
    }

    let variant = chosen_variant
        .ok_or_else(|| MathError::new("selected font radical construction has no variants"))?;
    let glyph = load_normalized_glyph_path(face, variant.variant_glyph, scale)?;
    let mut path = glyph.path;
    path.apply_affine(Affine::translate((
        f64::from(-glyph.x_min),
        f64::from(-glyph.y_min),
    )));

    Ok(RadicalOutline {
        path,
        height: glyph.y_max - glyph.y_min,
    })
}

struct FormulaLayouter<'a> {
    text: &'a mut MathTextEngine,
}

impl<'a> FormulaLayouter<'a> {
    fn new(text: &'a mut MathTextEngine) -> Self {
        Self { text }
    }

    fn layout(&mut self, node: &MathNode, font_size: f32) -> Result<FormulaLayout, MathError> {
        if !(font_size.is_finite() && font_size > 0.0) {
            return Err(MathError::new("font size must be positive and finite"));
        }
        self.layout_node(node, font_size)
    }

    fn layout_node(&mut self, node: &MathNode, font_size: f32) -> Result<FormulaLayout, MathError> {
        match node {
            MathNode::Row(nodes) => self.layout_row(nodes, font_size),
            MathNode::Symbol(text) => {
                let metrics = self.text.measure_text(text, font_size);
                Ok(FormulaLayout {
                    width: metrics.width,
                    ascent: metrics.ascent,
                    descent: metrics.descent,
                    commands: vec![DrawCommand::Text {
                        text: text.clone(),
                        x: 0.0,
                        baseline: 0.0,
                        font_size,
                    }],
                })
            }
            MathNode::Fraction {
                numerator,
                denominator,
            } => self.layout_fraction(numerator, denominator, font_size),
            MathNode::SupSub {
                base,
                superscript,
                subscript,
            } => self.layout_sup_sub(
                base,
                superscript.as_deref(),
                subscript.as_deref(),
                font_size,
            ),
            MathNode::Sqrt { radicand } => self.layout_sqrt(radicand, font_size),
        }
    }

    fn layout_row(
        &mut self,
        nodes: &[MathNode],
        font_size: f32,
    ) -> Result<FormulaLayout, MathError> {
        if nodes.is_empty() {
            return Ok(FormulaLayout::empty(font_size));
        }

        let mut children = Vec::with_capacity(nodes.len());
        for node in nodes {
            children.push(self.layout_node(node, font_size)?);
        }

        let ascent = children
            .iter()
            .map(|child| child.ascent)
            .fold(0.0_f32, f32::max);
        let descent = children
            .iter()
            .map(|child| child.descent)
            .fold(0.0_f32, f32::max);

        let mut width = 0.0;
        let mut commands = Vec::new();
        for child in children {
            commands.extend(
                child
                    .commands
                    .into_iter()
                    .map(|command| command.translated(width, 0.0)),
            );
            width += child.width;
        }

        Ok(FormulaLayout {
            width,
            ascent,
            descent,
            commands,
        })
    }

    fn layout_fraction(
        &mut self,
        numerator: &MathNode,
        denominator: &MathNode,
        font_size: f32,
    ) -> Result<FormulaLayout, MathError> {
        let script_size = (font_size * 0.84).max(6.0);
        let num = self.layout_node(numerator, script_size)?;
        let den = self.layout_node(denominator, script_size)?;

        let rule = (font_size * 0.06).max(1.0);
        let half_rule = rule * 0.5;
        let gap = font_size * 0.18;
        let side_padding = font_size * 0.12;

        let content_width = num.width.max(den.width);
        let width = content_width + side_padding * 2.0;

        let num_x = side_padding + (content_width - num.width) * 0.5;
        let den_x = side_padding + (content_width - den.width) * 0.5;

        let num_baseline = -(gap + half_rule + num.descent);
        let den_baseline = gap + half_rule + den.ascent;

        let mut commands = Vec::new();
        commands.extend(
            num.commands
                .into_iter()
                .map(|command| command.translated(num_x, num_baseline)),
        );
        commands.extend(
            den.commands
                .into_iter()
                .map(|command| command.translated(den_x, den_baseline)),
        );
        commands.push(DrawCommand::Rect {
            x: 0.0,
            y: -half_rule,
            width,
            height: rule,
        });

        let ascent = gap + half_rule + num.ascent + num.descent;
        let descent = gap + half_rule + den.ascent + den.descent;

        Ok(FormulaLayout {
            width,
            ascent,
            descent,
            commands,
        })
    }

    fn layout_sup_sub(
        &mut self,
        base: &MathNode,
        superscript: Option<&MathNode>,
        subscript: Option<&MathNode>,
        font_size: f32,
    ) -> Result<FormulaLayout, MathError> {
        let base_layout = self.layout_node(base, font_size)?;
        let script_size = (font_size * 0.72).max(5.0);
        let sup_layout = if let Some(sup) = superscript {
            Some(self.layout_node(sup, script_size)?)
        } else {
            None
        };
        let sub_layout = if let Some(sub) = subscript {
            Some(self.layout_node(sub, script_size)?)
        } else {
            None
        };

        let script_dx = base_layout.width + font_size * 0.06;
        let sup_baseline = sup_layout
            .as_ref()
            .map(|sup| -(base_layout.ascent * 0.62 + sup.descent + font_size * 0.05));
        let sub_baseline = sub_layout
            .as_ref()
            .map(|sub| base_layout.descent * 0.55 + sub.ascent + font_size * 0.05);

        let script_width = sup_layout
            .as_ref()
            .map(|layout| layout.width)
            .into_iter()
            .chain(sub_layout.as_ref().map(|layout| layout.width))
            .fold(0.0_f32, f32::max);
        let width = if script_width > 0.0 {
            script_dx + script_width
        } else {
            base_layout.width
        };

        let mut commands = Vec::new();
        commands.extend(base_layout.commands);

        if let Some(sup) = sup_layout {
            let dy = sup_baseline.expect("sup baseline must exist when superscript exists");
            commands.extend(
                sup.commands
                    .into_iter()
                    .map(|command| command.translated(script_dx, dy)),
            );
        }

        if let Some(sub) = sub_layout {
            let dy = sub_baseline.expect("sub baseline must exist when subscript exists");
            commands.extend(
                sub.commands
                    .into_iter()
                    .map(|command| command.translated(script_dx, dy)),
            );
        }

        let mut ascent = base_layout.ascent;
        if let (Some(sup), Some(sup_y)) = (superscript, sup_baseline) {
            let sup_layout = self.layout_node(sup, script_size)?;
            let top = sup_y - sup_layout.ascent;
            ascent = ascent.max(-top);
        }

        let mut descent = base_layout.descent;
        if let (Some(sub), Some(sub_y)) = (subscript, sub_baseline) {
            let sub_layout = self.layout_node(sub, script_size)?;
            let bottom = sub_y + sub_layout.descent;
            descent = descent.max(bottom);
        }

        Ok(FormulaLayout {
            width,
            ascent,
            descent,
            commands,
        })
    }

    fn layout_sqrt(
        &mut self,
        radicand: &MathNode,
        font_size: f32,
    ) -> Result<FormulaLayout, MathError> {
        let inner = self.layout_node(radicand, font_size)?;
        let root_font = self.text.resolve_font_data(RADICAL_GLYPH, font_size)?;
        let face = parse_face_from_font(&root_font)?;
        let scale = font_size / f32::from(face.units_per_em());
        let math = face
            .tables()
            .math
            .ok_or_else(|| MathError::new("selected radical font has no OpenType MATH table"))?;
        let constants = math
            .constants
            .ok_or_else(|| MathError::new("selected radical font MATH table has no constants"))?;
        let radical_glyph = face
            .glyph_index('√')
            .ok_or_else(|| MathError::new("selected radical font has no U+221A glyph"))?;

        let overbar_gap = (f32::from(constants.radical_vertical_gap().value) * scale).max(0.0);
        let stroke = (f32::from(constants.radical_rule_thickness().value) * scale)
            .max(font_size * 0.04)
            .max(1.0);
        let extra_ascender = (f32::from(constants.radical_extra_ascender().value) * scale).max(0.0);
        let target_advance = inner.ascent + inner.descent;
        let mut radical_outline =
            build_radical_outline_from_math(&face, radical_glyph, target_advance, scale)?;
        let desired_height = target_advance + extra_ascender;
        compress_radical_height_if_oversized(&mut radical_outline, desired_height)?;
        let line_ascent = inner
            .ascent
            .max(inner.ascent + overbar_gap + stroke + extra_ascender);
        let overbar_target_y = -(line_ascent - extra_ascender);
        let top_band_max = (stroke * 2.5)
            .max(radical_outline.height * 0.18)
            .min(radical_outline.height);
        let (_, overbar_join_x) = path_x_range_in_y_band(&radical_outline.path, 0.0, top_band_max)
            .ok_or_else(|| {
                MathError::new("failed to find radical top-bar x-range in OpenType outline")
            })?;
        let (bar_top_local, bar_bottom_local) = sample_radical_top_bar_band(
            &radical_outline.path,
            overbar_join_x,
            stroke,
        )
        .ok_or_else(|| {
            MathError::new("failed to detect radical top-bar vertical range near OpenType join")
        })?;
        let (bar_left_local, _) =
            path_x_range_in_y_band(&radical_outline.path, bar_top_local, bar_bottom_local)
                .ok_or_else(|| {
                    MathError::new("failed to detect radical top-bar horizontal span")
                })?;
        let overbar_height = (bar_bottom_local - bar_top_local)
            .clamp(stroke * 0.6, stroke * 1.25)
            .max(1.0);
        let root_top = overbar_target_y - bar_top_local;
        let root_bottom = root_top + radical_outline.height;
        let overbar_y = overbar_target_y;
        let radicand_inset = stroke * 0.18;
        let inner_x = (bar_left_local + radicand_inset)
            .max(overbar_join_x - inner.width)
            .max(0.0);
        let overlap = overbar_height * 0.28;
        let overbar_x = (overbar_join_x - overlap).max(0.0);
        let radicand_right = inner_x + inner.width;
        let overbar_width = (radicand_right - overbar_x).max(0.0);
        let width = radicand_right.max(overbar_join_x);
        let vertical_guard = (stroke * 0.25).max(1.0);
        let ascent = line_ascent.max(-root_top + vertical_guard);
        let descent = inner.descent.max(root_bottom + vertical_guard);

        let mut commands = Vec::new();
        if overbar_width > 0.0 {
            commands.push(DrawCommand::Rect {
                x: overbar_x,
                y: overbar_y,
                width: overbar_width,
                height: overbar_height,
            });
        }
        commands.extend(
            inner
                .commands
                .into_iter()
                .map(|command| command.translated(inner_x, 0.0)),
        );
        commands.push(DrawCommand::Path {
            path: radical_outline.path,
            x: 0.0,
            y: root_top,
        });

        Ok(FormulaLayout {
            width,
            ascent,
            descent,
            commands,
        })
    }
}

fn parse_math_source(format: MathInputFormat, source: &str) -> Result<MathNode, MathError> {
    match format {
        MathInputFormat::Latex => LatexParser::new(source).parse(),
        MathInputFormat::MathMl => MathMlParser::new(source).parse(),
        MathInputFormat::Typst => TypstParser::new(source).parse(),
    }
}

fn collapse_row(mut nodes: Vec<MathNode>) -> MathNode {
    match nodes.len() {
        0 => MathNode::Row(Vec::new()),
        1 => nodes
            .pop()
            .expect("single-element row must have one element"),
        _ => MathNode::Row(nodes),
    }
}

fn attach_script(
    nodes: &mut Vec<MathNode>,
    script: MathNode,
    is_superscript: bool,
) -> Result<(), MathError> {
    let base = nodes.pop().ok_or_else(|| {
        MathError::new(if is_superscript {
            "superscript marker '^' has no base expression"
        } else {
            "subscript marker '_' has no base expression"
        })
    })?;

    let updated = match base {
        MathNode::SupSub {
            base,
            superscript,
            subscript,
        } => {
            if is_superscript {
                if superscript.is_some() {
                    return Err(MathError::new("duplicate superscript for the same base"));
                }
                MathNode::SupSub {
                    base,
                    superscript: Some(Box::new(script)),
                    subscript,
                }
            } else {
                if subscript.is_some() {
                    return Err(MathError::new("duplicate subscript for the same base"));
                }
                MathNode::SupSub {
                    base,
                    superscript,
                    subscript: Some(Box::new(script)),
                }
            }
        }
        base => MathNode::SupSub {
            base: Box::new(base),
            superscript: is_superscript.then_some(Box::new(script.clone())),
            subscript: (!is_superscript).then_some(Box::new(script)),
        },
    };

    nodes.push(updated);
    Ok(())
}

#[derive(Debug)]
struct LatexParser {
    chars: Vec<char>,
    pos: usize,
}

impl LatexParser {
    fn new(source: &str) -> Self {
        Self {
            chars: source.chars().collect(),
            pos: 0,
        }
    }

    fn parse(mut self) -> Result<MathNode, MathError> {
        let nodes = self.parse_row(None)?;
        if self.peek().is_some() {
            return Err(MathError::new(format!(
                "unexpected trailing token '{}' at position {}",
                self.peek().unwrap_or('\0'),
                self.pos
            )));
        }
        Ok(collapse_row(nodes))
    }

    fn parse_row(&mut self, terminator: Option<char>) -> Result<Vec<MathNode>, MathError> {
        let mut nodes = Vec::new();

        loop {
            self.skip_whitespace();
            let Some(ch) = self.peek() else {
                break;
            };

            if Some(ch) == terminator {
                break;
            }

            match ch {
                '{' => {
                    self.consume();
                    let group = self.parse_group('}')?;
                    nodes.push(group);
                }
                '}' => {
                    return Err(MathError::new(format!(
                        "unexpected '}}' at position {}",
                        self.pos
                    )));
                }
                '^' => {
                    self.consume();
                    let script = self.parse_script_atom()?;
                    attach_script(&mut nodes, script, true)?;
                }
                '_' => {
                    self.consume();
                    let script = self.parse_script_atom()?;
                    attach_script(&mut nodes, script, false)?;
                }
                '\\' => {
                    self.consume();
                    let command = self.parse_command()?;
                    nodes.push(command);
                }
                _ => {
                    self.consume();
                    nodes.push(MathNode::Symbol(ch.to_string()));
                }
            }
        }

        Ok(nodes)
    }

    fn parse_group(&mut self, close: char) -> Result<MathNode, MathError> {
        let nodes = self.parse_row(Some(close))?;
        self.expect(close)?;
        Ok(collapse_row(nodes))
    }

    fn parse_script_atom(&mut self) -> Result<MathNode, MathError> {
        self.skip_whitespace();
        match self.peek() {
            Some('{') => {
                self.consume();
                self.parse_group('}')
            }
            Some('\\') => {
                self.consume();
                self.parse_command()
            }
            Some(ch) => {
                self.consume();
                Ok(MathNode::Symbol(ch.to_string()))
            }
            None => Err(MathError::new("missing script body after '^' or '_'")),
        }
    }

    fn parse_command(&mut self) -> Result<MathNode, MathError> {
        let name = self.consume_while(|ch| ch.is_ascii_alphabetic());
        if name.is_empty() {
            let escaped = self
                .consume()
                .ok_or_else(|| MathError::new("incomplete escape sequence"))?;
            return Ok(MathNode::Symbol(escaped.to_string()));
        }

        match name.as_str() {
            "frac" => {
                self.skip_whitespace();
                self.expect('{')?;
                let numerator = self.parse_group('}')?;
                self.skip_whitespace();
                self.expect('{')?;
                let denominator = self.parse_group('}')?;
                Ok(MathNode::Fraction {
                    numerator: Box::new(numerator),
                    denominator: Box::new(denominator),
                })
            }
            "sqrt" => {
                self.skip_whitespace();
                self.expect('{')?;
                let radicand = self.parse_group('}')?;
                Ok(MathNode::Sqrt {
                    radicand: Box::new(radicand),
                })
            }
            "left" | "right" => {
                self.skip_whitespace();
                let delimiter = self
                    .consume()
                    .ok_or_else(|| MathError::new("expected delimiter after \\left/\\right"))?;
                if delimiter == '.' {
                    Ok(MathNode::Row(Vec::new()))
                } else if delimiter == '\\' {
                    let escaped = self
                        .consume()
                        .ok_or_else(|| MathError::new("incomplete delimiter escape"))?;
                    Ok(MathNode::Symbol(escaped.to_string()))
                } else {
                    Ok(MathNode::Symbol(delimiter.to_string()))
                }
            }
            "mathrm" | "text" | "operatorname" => {
                self.skip_whitespace();
                self.expect('{')?;
                self.parse_group('}')
            }
            _ => map_math_symbol(name.as_str())
                .map(|symbol| MathNode::Symbol(symbol.to_string()))
                .ok_or_else(|| MathError::new(format!("unsupported LaTeX command '\\{}'", name))),
        }
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.consume();
        }
    }

    fn expect(&mut self, expected: char) -> Result<(), MathError> {
        let got = self.consume().ok_or_else(|| {
            MathError::new(format!("expected '{}' but reached end of input", expected))
        })?;

        if got != expected {
            return Err(MathError::new(format!(
                "expected '{}' at position {}, found '{}'",
                expected,
                self.pos.saturating_sub(1),
                got
            )));
        }
        Ok(())
    }

    fn peek(&self) -> Option<char> {
        self.chars.as_slice().get(self.pos).copied()
    }

    fn consume(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += 1;
        Some(ch)
    }

    fn consume_while<F>(&mut self, mut predicate: F) -> String
    where
        F: FnMut(char) -> bool,
    {
        let mut token = String::new();
        while let Some(ch) = self.peek() {
            if !predicate(ch) {
                break;
            }
            self.consume();
            token.push(ch);
        }
        token
    }
}

#[derive(Debug)]
struct TypstParser {
    chars: Vec<char>,
    pos: usize,
}

impl TypstParser {
    fn new(source: &str) -> Self {
        Self {
            chars: source.chars().collect(),
            pos: 0,
        }
    }

    fn parse(mut self) -> Result<MathNode, MathError> {
        let nodes = self.parse_row(&[])?;
        self.skip_whitespace();
        if self.peek().is_some() {
            return Err(MathError::new(format!(
                "unexpected trailing token '{}' at position {}",
                self.peek().unwrap_or('\0'),
                self.pos
            )));
        }
        Ok(collapse_row(nodes))
    }

    fn parse_row(&mut self, terminators: &[char]) -> Result<Vec<MathNode>, MathError> {
        let mut nodes = Vec::new();

        loop {
            self.skip_whitespace();
            let Some(ch) = self.peek() else {
                break;
            };
            if terminators.contains(&ch) {
                break;
            }

            match ch {
                '^' => {
                    self.consume();
                    let script = self.parse_script_atom()?;
                    attach_script(&mut nodes, script, true)?;
                }
                '_' => {
                    self.consume();
                    let script = self.parse_script_atom()?;
                    attach_script(&mut nodes, script, false)?;
                }
                '(' => {
                    self.consume();
                    let group = collapse_row(self.parse_row(&[')'])?);
                    self.expect(')')?;
                    nodes.push(group);
                }
                '{' => {
                    self.consume();
                    let group = collapse_row(self.parse_row(&['}'])?);
                    self.expect('}')?;
                    nodes.push(group);
                }
                ',' => break,
                ch if ch.is_ascii_alphabetic() => {
                    let ident = self.consume_while(|c| c.is_ascii_alphabetic());
                    if self.peek() == Some('(') {
                        nodes.push(self.parse_function(&ident)?);
                    } else if let Some(symbol) = map_math_symbol(ident.as_str()) {
                        nodes.push(MathNode::Symbol(symbol.to_string()));
                    } else {
                        nodes.push(MathNode::Symbol(ident));
                    }
                }
                ch if ch.is_ascii_digit() => {
                    let number = self.consume_while(|c| c.is_ascii_digit() || c == '.');
                    nodes.push(MathNode::Symbol(number));
                }
                _ => {
                    self.consume();
                    nodes.push(MathNode::Symbol(ch.to_string()));
                }
            }
        }

        Ok(nodes)
    }

    fn parse_function(&mut self, name: &str) -> Result<MathNode, MathError> {
        self.expect('(')?;
        match name {
            "frac" => {
                let numerator = collapse_row(self.parse_row(&[','])?);
                self.expect(',')?;
                let denominator = collapse_row(self.parse_row(&[')'])?);
                self.expect(')')?;
                Ok(MathNode::Fraction {
                    numerator: Box::new(numerator),
                    denominator: Box::new(denominator),
                })
            }
            "sqrt" => {
                let body = collapse_row(self.parse_row(&[')'])?);
                self.expect(')')?;
                Ok(MathNode::Sqrt {
                    radicand: Box::new(body),
                })
            }
            _ => Err(MathError::new(format!(
                "unsupported Typst function '{}'",
                name
            ))),
        }
    }

    fn parse_script_atom(&mut self) -> Result<MathNode, MathError> {
        self.skip_whitespace();
        match self.peek() {
            Some('{') => {
                self.consume();
                let row = collapse_row(self.parse_row(&['}'])?);
                self.expect('}')?;
                Ok(row)
            }
            Some('(') => {
                self.consume();
                let row = collapse_row(self.parse_row(&[')'])?);
                self.expect(')')?;
                Ok(row)
            }
            Some(ch) if ch.is_ascii_alphabetic() => {
                let ident = self.consume_while(|c| c.is_ascii_alphabetic());
                Ok(MathNode::Symbol(
                    map_math_symbol(ident.as_str())
                        .unwrap_or(ident.as_str())
                        .to_string(),
                ))
            }
            Some(ch) if ch.is_ascii_digit() => {
                let number = self.consume_while(|c| c.is_ascii_digit() || c == '.');
                Ok(MathNode::Symbol(number))
            }
            Some(ch) => {
                self.consume();
                Ok(MathNode::Symbol(ch.to_string()))
            }
            None => Err(MathError::new("missing script body in Typst expression")),
        }
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.consume();
        }
    }

    fn expect(&mut self, expected: char) -> Result<(), MathError> {
        let actual = self
            .consume()
            .ok_or_else(|| MathError::new(format!("expected '{}' but input ended", expected)))?;
        if actual != expected {
            return Err(MathError::new(format!(
                "expected '{}' at position {}, found '{}'",
                expected,
                self.pos.saturating_sub(1),
                actual
            )));
        }
        Ok(())
    }

    fn peek(&self) -> Option<char> {
        self.chars.as_slice().get(self.pos).copied()
    }

    fn consume(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += 1;
        Some(ch)
    }

    fn consume_while<F>(&mut self, mut predicate: F) -> String
    where
        F: FnMut(char) -> bool,
    {
        let mut out = String::new();
        while let Some(ch) = self.peek() {
            if !predicate(ch) {
                break;
            }
            self.consume();
            out.push(ch);
        }
        out
    }
}

#[derive(Debug)]
struct MathMlParser<'a> {
    source: &'a str,
    pos: usize,
}

impl<'a> MathMlParser<'a> {
    fn new(source: &'a str) -> Self {
        Self { source, pos: 0 }
    }

    fn parse(mut self) -> Result<MathNode, MathError> {
        self.skip_ws();
        let node = self.parse_element()?;
        self.skip_ws();
        if self.pos != self.source.len() {
            return Err(MathError::new(format!(
                "unexpected trailing MathML at byte offset {}",
                self.pos
            )));
        }
        Ok(node)
    }

    fn parse_element(&mut self) -> Result<MathNode, MathError> {
        let (name, self_closing) = self.parse_open_tag()?;
        if self_closing {
            return Err(MathError::new(format!(
                "self-closing MathML tag '{}' is not supported",
                name
            )));
        }

        match strip_namespace(name.as_str()) {
            "math" | "mrow" => {
                let children = self.parse_children(name.as_str())?;
                Ok(collapse_row(children))
            }
            "mi" | "mn" | "mo" | "mtext" => {
                let text = self.read_text_until_close(name.as_str())?;
                let decoded = decode_entities(text.trim())?;
                if decoded.is_empty() {
                    return Err(MathError::new(format!(
                        "MathML tag '{}' cannot be empty",
                        name
                    )));
                }
                Ok(MathNode::Symbol(decoded))
            }
            "mfrac" => {
                let children = self.parse_children(name.as_str())?;
                if children.len() != 2 {
                    return Err(MathError::new(format!(
                        "MathML <mfrac> expects exactly 2 children, got {}",
                        children.len()
                    )));
                }
                let mut iter = children.into_iter();
                let numerator = iter.next().expect("mfrac child one must exist");
                let denominator = iter.next().expect("mfrac child two must exist");
                Ok(MathNode::Fraction {
                    numerator: Box::new(numerator),
                    denominator: Box::new(denominator),
                })
            }
            "msup" => {
                let children = self.parse_children(name.as_str())?;
                if children.len() != 2 {
                    return Err(MathError::new(format!(
                        "MathML <msup> expects exactly 2 children, got {}",
                        children.len()
                    )));
                }
                let mut iter = children.into_iter();
                Ok(MathNode::SupSub {
                    base: Box::new(iter.next().expect("msup base must exist")),
                    superscript: Some(Box::new(iter.next().expect("msup superscript must exist"))),
                    subscript: None,
                })
            }
            "msub" => {
                let children = self.parse_children(name.as_str())?;
                if children.len() != 2 {
                    return Err(MathError::new(format!(
                        "MathML <msub> expects exactly 2 children, got {}",
                        children.len()
                    )));
                }
                let mut iter = children.into_iter();
                Ok(MathNode::SupSub {
                    base: Box::new(iter.next().expect("msub base must exist")),
                    superscript: None,
                    subscript: Some(Box::new(iter.next().expect("msub subscript must exist"))),
                })
            }
            "msubsup" => {
                let children = self.parse_children(name.as_str())?;
                if children.len() != 3 {
                    return Err(MathError::new(format!(
                        "MathML <msubsup> expects exactly 3 children, got {}",
                        children.len()
                    )));
                }
                let mut iter = children.into_iter();
                let base = iter.next().expect("msubsup base must exist");
                let sub = iter.next().expect("msubsup subscript child must exist");
                let sup = iter.next().expect("msubsup superscript child must exist");
                Ok(MathNode::SupSub {
                    base: Box::new(base),
                    superscript: Some(Box::new(sup)),
                    subscript: Some(Box::new(sub)),
                })
            }
            "msqrt" => {
                let children = self.parse_children(name.as_str())?;
                if children.is_empty() {
                    return Err(MathError::new("MathML <msqrt> requires content"));
                }
                Ok(MathNode::Sqrt {
                    radicand: Box::new(collapse_row(children)),
                })
            }
            other => Err(MathError::new(format!(
                "unsupported MathML tag '{}'",
                other
            ))),
        }
    }

    fn parse_children(&mut self, parent_tag: &str) -> Result<Vec<MathNode>, MathError> {
        let mut children = Vec::new();
        loop {
            self.skip_ws();
            if self.starts_with("</") {
                let close_name = self.parse_close_tag()?;
                if strip_namespace(close_name.as_str()) != strip_namespace(parent_tag) {
                    return Err(MathError::new(format!(
                        "mismatched close tag: expected </{}> but found </{}>",
                        parent_tag, close_name
                    )));
                }
                break;
            }

            if self.peek_byte() == Some(b'<') {
                children.push(self.parse_element()?);
            } else {
                let text = self.read_text();
                let decoded = decode_entities(text.trim())?;
                if !decoded.is_empty() {
                    children.push(MathNode::Symbol(decoded));
                }
            }
        }
        Ok(children)
    }

    fn read_text_until_close(&mut self, parent_tag: &str) -> Result<String, MathError> {
        let mut text = String::new();
        loop {
            if self.starts_with("</") {
                let close_name = self.parse_close_tag()?;
                if strip_namespace(close_name.as_str()) != strip_namespace(parent_tag) {
                    return Err(MathError::new(format!(
                        "mismatched close tag: expected </{}> but found </{}>",
                        parent_tag, close_name
                    )));
                }
                break;
            }
            if self.peek_byte() == Some(b'<') {
                return Err(MathError::new(format!(
                    "MathML tag '{}' must contain text only",
                    parent_tag
                )));
            }
            text.push_str(&self.read_text());
            if self.pos >= self.source.len() {
                return Err(MathError::new(format!(
                    "unterminated MathML tag '{}'",
                    parent_tag
                )));
            }
        }
        Ok(text)
    }

    fn parse_open_tag(&mut self) -> Result<(String, bool), MathError> {
        self.expect_bytes("<")?;
        if self.peek_byte() == Some(b'/') {
            return Err(MathError::new(format!(
                "unexpected close tag at byte offset {}",
                self.pos
            )));
        }

        let name = self.parse_tag_name()?;
        let mut self_closing = false;

        loop {
            let Some(byte) = self.peek_byte() else {
                return Err(MathError::new("unterminated opening tag"));
            };

            match byte {
                b'>' => {
                    self.pos += 1;
                    break;
                }
                b'/' => {
                    if self.peek_next_byte() == Some(b'>') {
                        self.pos += 2;
                        self_closing = true;
                        break;
                    }
                    self.pos += 1;
                }
                b'"' | b'\'' => {
                    let quote = byte;
                    self.pos += 1;
                    while let Some(next) = self.peek_byte() {
                        self.pos += 1;
                        if next == quote {
                            break;
                        }
                    }
                }
                _ => self.pos += 1,
            }
        }

        Ok((name, self_closing))
    }

    fn parse_close_tag(&mut self) -> Result<String, MathError> {
        self.expect_bytes("</")?;
        let name = self.parse_tag_name()?;
        self.skip_ws();
        self.expect_bytes(">")?;
        Ok(name)
    }

    fn parse_tag_name(&mut self) -> Result<String, MathError> {
        let start = self.pos;
        while let Some(byte) = self.peek_byte() {
            if byte.is_ascii_alphanumeric() || byte == b':' || byte == b'-' {
                self.pos += 1;
            } else {
                break;
            }
        }

        if self.pos == start {
            return Err(MathError::new(format!(
                "expected tag name at byte offset {}",
                self.pos
            )));
        }

        Ok(self.source[start..self.pos].to_string())
    }

    fn read_text(&mut self) -> String {
        let start = self.pos;
        while let Some(byte) = self.peek_byte() {
            if byte == b'<' {
                break;
            }
            self.pos += 1;
        }
        self.source[start..self.pos].to_string()
    }

    fn skip_ws(&mut self) {
        while self
            .peek_byte()
            .is_some_and(|byte| char::from(byte).is_whitespace())
        {
            self.pos += 1;
        }
    }

    fn expect_bytes(&mut self, expected: &str) -> Result<(), MathError> {
        if !self.starts_with(expected) {
            return Err(MathError::new(format!(
                "expected '{}' at byte offset {}",
                expected, self.pos
            )));
        }
        self.pos += expected.len();
        Ok(())
    }

    fn starts_with(&self, needle: &str) -> bool {
        self.source[self.pos..].starts_with(needle)
    }

    fn peek_byte(&self) -> Option<u8> {
        self.source.as_bytes().get(self.pos).copied()
    }

    fn peek_next_byte(&self) -> Option<u8> {
        self.source.as_bytes().get(self.pos + 1).copied()
    }
}

fn map_math_symbol(name: &str) -> Option<&'static str> {
    match name {
        "alpha" => Some("α"),
        "beta" => Some("β"),
        "gamma" => Some("γ"),
        "delta" => Some("δ"),
        "epsilon" | "varepsilon" => Some("ε"),
        "theta" => Some("θ"),
        "lambda" => Some("λ"),
        "mu" => Some("μ"),
        "pi" => Some("π"),
        "sigma" => Some("σ"),
        "phi" | "varphi" => Some("φ"),
        "omega" => Some("ω"),
        "Gamma" => Some("Γ"),
        "Delta" => Some("Δ"),
        "Theta" => Some("Θ"),
        "Lambda" => Some("Λ"),
        "Pi" => Some("Π"),
        "Sigma" => Some("Σ"),
        "Phi" => Some("Φ"),
        "Omega" => Some("Ω"),
        "infty" => Some("∞"),
        "sum" => Some("∑"),
        "prod" => Some("∏"),
        "int" => Some("∫"),
        "partial" => Some("∂"),
        "nabla" => Some("∇"),
        "times" => Some("×"),
        "cdot" => Some("·"),
        "pm" => Some("±"),
        "neq" => Some("≠"),
        "le" | "leq" => Some("≤"),
        "ge" | "geq" => Some("≥"),
        "approx" => Some("≈"),
        "to" | "rightarrow" => Some("→"),
        "leftarrow" => Some("←"),
        "Rightarrow" => Some("⇒"),
        "Leftarrow" => Some("⇐"),
        "iff" | "Leftrightarrow" => Some("⇔"),
        "in" => Some("∈"),
        "notin" => Some("∉"),
        "subset" => Some("⊂"),
        "subseteq" => Some("⊆"),
        "supset" => Some("⊃"),
        "supseteq" => Some("⊇"),
        "cup" => Some("∪"),
        "cap" => Some("∩"),
        "sin" => Some("sin"),
        "cos" => Some("cos"),
        "tan" => Some("tan"),
        "log" => Some("log"),
        "ln" => Some("ln"),
        _ => None,
    }
}

fn decode_entities(text: &str) -> Result<String, MathError> {
    if !text.contains('&') {
        return Ok(text.to_string());
    }

    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '&' {
            out.push(ch);
            continue;
        }

        let mut entity = String::new();
        while let Some(next) = chars.peek().copied() {
            chars.next();
            if next == ';' {
                break;
            }
            entity.push(next);
        }

        if entity.is_empty() {
            return Err(MathError::new("empty XML entity"));
        }

        let decoded = match entity.as_str() {
            "lt" => "<".to_string(),
            "gt" => ">".to_string(),
            "amp" => "&".to_string(),
            "quot" => "\"".to_string(),
            "apos" => "'".to_string(),
            "alpha" => "α".to_string(),
            "beta" => "β".to_string(),
            "gamma" => "γ".to_string(),
            "delta" => "δ".to_string(),
            "theta" => "θ".to_string(),
            "pi" => "π".to_string(),
            "sigma" => "σ".to_string(),
            "phi" => "φ".to_string(),
            "omega" => "ω".to_string(),
            raw if raw.starts_with("#x") => {
                let value = u32::from_str_radix(&raw[2..], 16)
                    .map_err(|_| MathError::new(format!("invalid hex entity '&{};'", raw)))?;
                char::from_u32(value)
                    .ok_or_else(|| MathError::new(format!("invalid unicode entity '&{};'", raw)))?
                    .to_string()
            }
            raw if raw.starts_with('#') => {
                let value = raw[1..]
                    .parse::<u32>()
                    .map_err(|_| MathError::new(format!("invalid decimal entity '&{};'", raw)))?;
                char::from_u32(value)
                    .ok_or_else(|| MathError::new(format!("invalid unicode entity '&{};'", raw)))?
                    .to_string()
            }
            raw => {
                return Err(MathError::new(format!(
                    "unsupported XML entity '&{};'",
                    raw
                )));
            }
        };

        out.push_str(&decoded);
    }

    Ok(out)
}

fn strip_namespace(tag: &str) -> &str {
    tag.rsplit_once(':').map_or(tag, |(_, name)| name)
}

fn resolve_math_font_stack(requested: &str) -> String {
    let requested = requested.trim();
    if requested.is_empty() {
        return DEFAULT_MATH_FONT_STACK.to_string();
    }
    if requested.contains(',') {
        return requested.to_string();
    }
    format!("{requested}, {DEFAULT_MATH_FONT_STACK}")
}

fn resolve_math_color(env: &Environment, explicit: Option<&Color>) -> PenikoColor {
    let resolved = explicit
        .map(|color| color.resolve(env).get())
        .or_else(|| env.query::<ForegroundColor, ResolvedColor>().copied())
        .unwrap_or_else(|| ResolvedColor::from_srgb(Srgb::WHITE));

    resolved_color_to_peniko(&resolved)
}

fn resolved_color_to_peniko(color: &ResolvedColor) -> PenikoColor {
    let srgb = color.to_srgb();
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let r = (srgb.red * 255.0).clamp(0.0, 255.0) as u8;
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let g = (srgb.green * 255.0).clamp(0.0, 255.0) as u8;
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let b = (srgb.blue * 255.0).clamp(0.0, 255.0) as u8;
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let a = (color.opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    PenikoColor::from_rgba8(r, g, b, a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use waterui_graphics::{
        GpuContext, GpuFrame, GpuSurface, GpuView, OffscreenRenderConfig, OffscreenRenderOutput,
        OffscreenSize, VelloScene2D, wgpu,
    };

    struct MathOffscreenRenderer {
        scene: vello::Scene,
        renderer: Option<vello::Renderer>,
        intermediate_texture: Option<wgpu::Texture>,
        intermediate_view: Option<wgpu::TextureView>,
        blit_pipeline: Option<wgpu::RenderPipeline>,
        bind_group_layout: Option<wgpu::BindGroupLayout>,
        sampler: Option<wgpu::Sampler>,
        size: (u32, u32),
        background: wgpu::Color,
    }

    impl core::fmt::Debug for MathOffscreenRenderer {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("MathOffscreenRenderer")
                .field("size", &self.size)
                .finish_non_exhaustive()
        }
    }

    impl MathOffscreenRenderer {
        fn new(scene: vello::Scene, background: wgpu::Color) -> Self {
            Self {
                scene,
                renderer: None,
                intermediate_texture: None,
                intermediate_view: None,
                blit_pipeline: None,
                bind_group_layout: None,
                sampler: None,
                size: (0, 0),
                background,
            }
        }

        fn ensure_intermediate_texture(&mut self, device: &wgpu::Device, width: u32, height: u32) {
            if self.size == (width, height) {
                return;
            }

            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("math_offscreen_intermediate"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            self.intermediate_view =
                Some(texture.create_view(&wgpu::TextureViewDescriptor::default()));
            self.intermediate_texture = Some(texture);
            self.size = (width, height);
        }
    }

    impl GpuView for MathOffscreenRenderer {
        async fn setup(&mut self, ctx: &GpuContext<'_>, _env: &mut waterui_core::Environment) {
            self.renderer = Some(
                vello::Renderer::new(
                    ctx.device,
                    vello::RendererOptions {
                        use_cpu: false,
                        antialiasing_support: vello::AaSupport::area_only(),
                        num_init_threads: std::num::NonZeroUsize::new(1),
                        pipeline_cache: None,
                    },
                )
                .expect("failed to create math offscreen vello renderer"),
            );

            let shader = ctx
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(waterui_graphics::shaders::BLIT.label),
                    source: wgpu::ShaderSource::Wgsl(waterui_graphics::shaders::BLIT.source.into()),
                });
            let bind_group_layout =
                ctx.device
                    .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        label: Some("math_offscreen_blit_bind_group_layout"),
                        entries: &[
                            wgpu::BindGroupLayoutEntry {
                                binding: 0,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Texture {
                                    sample_type: wgpu::TextureSampleType::Float {
                                        filterable: true,
                                    },
                                    view_dimension: wgpu::TextureViewDimension::D2,
                                    multisampled: false,
                                },
                                count: None,
                            },
                            wgpu::BindGroupLayoutEntry {
                                binding: 1,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                                count: None,
                            },
                        ],
                    });
            let pipeline_layout =
                ctx.device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("math_offscreen_blit_pipeline_layout"),
                        bind_group_layouts: &[&bind_group_layout],
                        push_constant_ranges: &[],
                    });
            let blend = if ctx.is_hdr() {
                None
            } else {
                Some(wgpu::BlendState::ALPHA_BLENDING)
            };
            self.blit_pipeline = Some(ctx.device.create_render_pipeline(
                &wgpu::RenderPipelineDescriptor {
                    label: Some("math_offscreen_blit_pipeline"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_main"),
                        buffers: &[],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_main"),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: ctx.surface_format,
                            blend,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: Default::default(),
                    }),
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview: None,
                    cache: ctx.pipeline_cache,
                },
            ));
            self.sampler = Some(ctx.device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("math_offscreen_blit_sampler"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }));
            self.bind_group_layout = Some(bind_group_layout);
        }

        fn render(&mut self, frame: &mut GpuFrame) {
            self.ensure_intermediate_texture(frame.device, frame.width, frame.height);
            let intermediate_view = self
                .intermediate_view
                .as_ref()
                .expect("math offscreen intermediate view missing");

            self.renderer
                .as_mut()
                .expect("math offscreen renderer used before setup")
                .render_to_texture(
                    frame.device,
                    frame.queue,
                    &self.scene,
                    intermediate_view,
                    &vello::RenderParams {
                        base_color: peniko::Color::TRANSPARENT,
                        width: frame.width,
                        height: frame.height,
                        antialiasing_method: vello::AaConfig::Area,
                    },
                )
                .expect("math offscreen vello render failed");

            let bind_group_layout = self
                .bind_group_layout
                .as_ref()
                .expect("math offscreen bind group layout missing");
            let sampler = self
                .sampler
                .as_ref()
                .expect("math offscreen sampler missing");
            let bind_group = frame.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("math_offscreen_blit_bind_group"),
                layout: bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(intermediate_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            });

            let mut encoder =
                frame
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("math_offscreen_blit_encoder"),
                    });
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("math_offscreen_blit_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &frame.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(self.background),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_pipeline(
                    self.blit_pipeline
                        .as_ref()
                        .expect("math offscreen blit pipeline missing"),
                );
                pass.set_bind_group(0, &bind_group, &[]);
                pass.draw(0..6, 0..1);
            }
            frame.queue.submit([encoder.finish()]);
        }
    }

    waterui_graphics::impl_gpu_subview!(MathOffscreenRenderer);

    fn build_latex_scene(
        source: &str,
        font_size: f32,
        padding: f32,
        canvas: Option<(u32, u32)>,
    ) -> (vello::Scene, u32, u32) {
        let ast = parse_math_source(MathInputFormat::Latex, source)
            .expect("latex expression should parse for offscreen test");
        let mut text_engine = MathTextEngine::new("STIX Two Math");
        let mut layouter = FormulaLayouter::new(&mut text_engine);
        let layout = layouter
            .layout(&ast, font_size)
            .expect("layout should succeed for offscreen test");
        let mut content = MathSceneContent::new(
            layout,
            PenikoColor::from_rgba8(0, 0, 0, 255),
            "STIX Two Math",
            padding,
        );
        let (width, height) = content.intrinsic_size();
        let (pixel_width, pixel_height) = if let Some((canvas_width, canvas_height)) = canvas {
            (canvas_width, canvas_height)
        } else {
            #[allow(clippy::cast_possible_truncation)]
            let pixel_width = width.ceil().max(1.0) as u32;
            #[allow(clippy::cast_possible_truncation)]
            let pixel_height = height.ceil().max(1.0) as u32;
            (pixel_width, pixel_height)
        };

        let mut scene = vello::Scene::new();
        let mut scene2d = VelloScene2D::new(&mut scene);
        let needs_next_frame =
            content.build_scene(&mut scene2d, pixel_width as f32, pixel_height as f32);
        assert!(
            !needs_next_frame,
            "static math scene should not request another frame"
        );
        (scene, pixel_width, pixel_height)
    }

    fn render_latex_offscreen(
        source: &str,
        font_size: f32,
        padding: f32,
        canvas: Option<(u32, u32)>,
    ) -> OffscreenRenderOutput {
        let (scene, width, height) = build_latex_scene(source, font_size, padding, canvas);
        let renderer = MathOffscreenRenderer::new(scene, wgpu::Color::WHITE);
        let size =
            OffscreenSize::try_from_pixels(width, height).expect("offscreen size must be valid");
        let config = OffscreenRenderConfig::new(size).format(wgpu::TextureFormat::Rgba8Unorm);
        let mut env = waterui_core::Environment::new();
        GpuSurface::new(renderer)
            .render_offscreen(config, &mut env)
            .expect("math offscreen render should succeed")
    }

    const COMMON_LLM_LATEX: &[&str] = &[
        r"\frac{1+\sqrt{x}}{1+x^2}",
        r"e^{i\pi}+1=0",
        r"\sum_{i=1}^{n}i=\frac{n(n+1)}{2}",
        r"\int_0^\infty e^{-x}dx=1",
        r"\alpha+\beta=\gamma",
        r"\lambda^2-\mu^2=(\lambda-\mu)(\lambda+\mu)",
        r"\partial_x f=2x",
        r"\nabla f=(\partial_x f,\partial_y f)",
        r"\sin^2\theta+\cos^2\theta=1",
        r"\log(ab)=\log a+\log b",
        r"\ln(e^x)=x",
        r"x_{n+1}=x_n+\frac{1}{x_n}",
        r"\prod_{k=1}^{n}k=n!",
        r"A\subseteq B\Rightarrow A\cup C\subseteq B\cup C",
        r"A\cap(B\cup C)=(A\cap B)\cup(A\cap C)",
        r"x\neq y\Rightarrow x-y\neq 0",
        r"\phi=\frac{1+\sqrt{5}}{2}",
        r"\Delta x\to 0",
        r"f(x)=\frac{\sin x}{x}",
        r"\left(\frac{a}{b}\right)^2=\frac{a^2}{b^2}",
        r"\sqrt{\frac{a+b}{c+d}}",
        r"\Omega=\frac{2\pi}{T}",
        r"x^{\alpha+\beta}",
        r"y_{m+n}=y_my_n",
        r"\frac{\partial}{\partial x}x^2=2x",
        r"\sum_{k=0}^{n}\frac{1}{2^k}",
        r"p\in A\Rightarrow p\notin B",
        r"A\supseteq B\Leftrightarrow B\subseteq A",
    ];

    #[derive(Debug, Clone, Copy)]
    struct ForegroundBounds {
        min_x: u32,
        min_y: u32,
        max_x: u32,
        max_y: u32,
    }

    fn is_foreground_pixel(px: &[u8]) -> bool {
        px[0] < 245 || px[1] < 245 || px[2] < 245
    }

    fn foreground_bounds(output: &OffscreenRenderOutput) -> Option<ForegroundBounds> {
        let mut min_x = u32::MAX;
        let mut min_y = u32::MAX;
        let mut max_x = 0_u32;
        let mut max_y = 0_u32;
        let mut found = false;

        for y in 0..output.height {
            for x in 0..output.width {
                let pixel_index = (y * output.width + x) as usize * 4;
                let pixel = &output.rgba8[pixel_index..pixel_index + 4];
                if !is_foreground_pixel(pixel) {
                    continue;
                }
                found = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }

        found.then_some(ForegroundBounds {
            min_x,
            min_y,
            max_x,
            max_y,
        })
    }

    #[derive(Debug, Clone, Copy)]
    struct RectCommand {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    }

    fn layout_latex_for_test(source: &str, font_size: f32) -> FormulaLayout {
        let ast = parse_math_source(MathInputFormat::Latex, source).expect("latex parse");
        let mut text_engine = MathTextEngine::new("STIX Two Math");
        let mut layouter = FormulaLayouter::new(&mut text_engine);
        layouter.layout(&ast, font_size).expect("layout")
    }

    fn extract_rect_commands(layout: &FormulaLayout) -> Vec<RectCommand> {
        layout
            .commands
            .iter()
            .filter_map(|command| match command {
                DrawCommand::Rect {
                    x,
                    y,
                    width,
                    height,
                } => Some(RectCommand {
                    x: *x,
                    y: *y,
                    width: *width,
                    height: *height,
                }),
                _ => None,
            })
            .collect()
    }

    fn radical_path_command(layout: &FormulaLayout) -> (&kurbo::BezPath, f32, f32) {
        layout
            .commands
            .iter()
            .find_map(|command| match command {
                DrawCommand::Path { path, x, y } => Some((path, *x, *y)),
                _ => None,
            })
            .expect("layout must contain radical path command")
    }

    fn first_text_command_x(layout: &FormulaLayout) -> f32 {
        layout
            .commands
            .iter()
            .find_map(|command| match command {
                DrawCommand::Text { x, .. } => Some(*x),
                _ => None,
            })
            .expect("layout must contain at least one text command")
    }

    fn is_foreground_pixel_at(output: &OffscreenRenderOutput, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 || x >= output.width as i32 || y >= output.height as i32 {
            return false;
        }
        let pixel_index = (y as u32 * output.width + x as u32) as usize * 4;
        is_foreground_pixel(&output.rgba8[pixel_index..pixel_index + 4])
    }

    fn has_foreground_in_window(
        output: &OffscreenRenderOutput,
        x0: i32,
        x1: i32,
        y0: i32,
        y1: i32,
    ) -> bool {
        for y in y0..=y1 {
            for x in x0..=x1 {
                if is_foreground_pixel_at(output, x, y) {
                    return true;
                }
            }
        }
        false
    }

    #[test]
    fn parses_latex_fraction_with_scripts() {
        let ast = parse_math_source(MathInputFormat::Latex, r"\frac{a_1}{b^2}")
            .expect("latex parse should succeed");
        if let MathNode::Fraction {
            numerator,
            denominator,
        } = ast
        {
            assert!(matches!(*numerator, MathNode::SupSub { .. }));
            assert!(matches!(*denominator, MathNode::SupSub { .. }));
        } else {
            panic!("expected fraction AST");
        }
    }

    #[test]
    fn parses_typst_functions() {
        let ast = parse_math_source(MathInputFormat::Typst, "frac(alpha, sqrt(x_2))")
            .expect("typst parse should succeed");
        assert!(matches!(ast, MathNode::Fraction { .. }));
    }

    #[test]
    fn parses_mathml_fraction() {
        let ast = parse_math_source(
            MathInputFormat::MathMl,
            "<math><mfrac><mi>a</mi><msup><mi>b</mi><mn>2</mn></msup></mfrac></math>",
        )
        .expect("mathml parse should succeed");
        assert!(matches!(ast, MathNode::Fraction { .. }));
    }

    #[test]
    fn rejects_unknown_latex_command() {
        let result = parse_math_source(MathInputFormat::Latex, r"\unknown{x}");
        assert!(result.is_err(), "unsupported command must fail fast");
    }

    #[test]
    fn lays_out_fraction_with_positive_size() {
        let ast = parse_math_source(MathInputFormat::Latex, r"\frac{1}{2}").expect("parse");
        let mut text_engine = MathTextEngine::new("STIX Two Math");
        let mut layouter = FormulaLayouter::new(&mut text_engine);
        let layout = layouter.layout(&ast, 22.0).expect("layout");

        assert!(layout.width > 0.0);
        assert!(layout.height() > 0.0);
        assert!(
            layout
                .commands
                .iter()
                .any(|cmd| matches!(cmd, DrawCommand::Rect { .. }))
        );
    }

    #[test]
    fn extends_sqrt_for_tall_radicand() {
        let layout = layout_latex_for_test(r"\sqrt{\frac{1}{x^2+1}}", 22.0);

        let path_count = layout
            .commands
            .iter()
            .filter(|command| matches!(command, DrawCommand::Path { .. }))
            .count();
        let rect_count = layout
            .commands
            .iter()
            .filter(|command| matches!(command, DrawCommand::Rect { .. }))
            .count();

        assert!(
            path_count >= 1,
            "sqrt with tall radicand should emit at least one outline path, got {path_count}"
        );
        assert!(
            rect_count >= 1,
            "sqrt with tall radicand should emit an overbar, got {rect_count} rects"
        );
        assert!(
            layout.commands.iter().all(|command| !matches!(
                command,
                DrawCommand::Text { text, .. } if text == RADICAL_GLYPH
            )),
            "sqrt radical should come from OpenType MATH outlines instead of raw text glyph"
        );
    }

    #[test]
    fn sqrt_overbar_joins_radical_outline() {
        const FONT_SIZE: f32 = 30.0;
        let layout = layout_latex_for_test(r"\sqrt{\frac{a+b}{c+d}}", FONT_SIZE);
        let mut rects = extract_rect_commands(&layout);
        assert!(
            rects.len() >= 2,
            "sqrt fraction layout should have overbar and fraction bar"
        );
        rects.sort_by(|a, b| a.y.total_cmp(&b.y));
        let overbar = rects[0];
        let (radical_path, radical_x, radical_y) = radical_path_command(&layout);
        let radical_right = path_rightmost_x_in_y_band(
            radical_path,
            overbar.y - radical_y,
            overbar.y + overbar.height - radical_y,
        )
        .expect("radical path must intersect sqrt overbar y-band");
        let join_gap = overbar.x - (radical_x + radical_right);
        let max_leading = FONT_SIZE * 0.14;
        let tolerance = overbar.height * 0.6;
        assert!(
            join_gap >= -tolerance && join_gap <= max_leading + tolerance,
            "sqrt overbar/radical join is unreasonable: gap={join_gap}, max_leading={max_leading}, tolerance={tolerance}, overbar={overbar:?}, radical_right={radical_right}, radical_y={radical_y}"
        );
    }

    #[test]
    fn sqrt_overbar_length_tracks_radicand_width() {
        const FONT_SIZE: f32 = 36.0;
        let layout = layout_latex_for_test(r"\sqrt{5}", FONT_SIZE);
        let rects = extract_rect_commands(&layout);
        assert!(
            rects.len() <= 1,
            "simple sqrt should emit at most one explicit overbar extension, got {} rects: {:?}",
            rects.len(),
            rects
        );
        let radicand_layout = layout_latex_for_test("5", FONT_SIZE);
        let radicand_text_x = first_text_command_x(&layout);
        if let Some(overbar) = rects.first().copied() {
            let max_allowed = radicand_layout.width + FONT_SIZE * 0.5;
            let min_allowed = (radicand_layout.width * 0.5).max(1.0);
            assert!(
                overbar.width >= min_allowed && overbar.width <= max_allowed,
                "sqrt overbar width is unreasonable for radicand: overbar_width={}, radicand_width={}, allowed=[{}, {}]",
                overbar.width,
                radicand_layout.width,
                min_allowed,
                max_allowed
            );
            let leading = radicand_text_x - overbar.x;
            let max_leading = FONT_SIZE * 0.20 + overbar.height;
            assert!(
                leading <= max_leading,
                "sqrt overbar leads radicand too much: leading={leading}, max_leading={max_leading}, overbar={overbar:?}, text_x={radicand_text_x}"
            );
        } else {
            assert!(
                radicand_text_x > 0.0,
                "sqrt radicand should not start at x=0 when overbar comes from radical glyph"
            );
        }
    }

    #[test]
    fn sqrt_fraction_overbar_covers_fraction_rule() {
        const FONT_SIZE: f32 = 30.0;
        let layout = layout_latex_for_test(r"\sqrt{\frac{a+b}{c+d}}", FONT_SIZE);
        let mut rects = extract_rect_commands(&layout);
        assert!(
            !rects.is_empty(),
            "sqrt fraction layout should include at least the fraction bar"
        );
        rects.sort_by(|a, b| a.y.total_cmp(&b.y));
        let fraction_bar = rects[rects.len() - 1];
        let extension_bar = (rects.len() > 1).then_some(rects[0]);
        let (radical_path, radical_x, radical_y) = radical_path_command(&layout);
        let top_bar_min_y = extension_bar
            .map(|bar| bar.y)
            .unwrap_or(fraction_bar.y - fraction_bar.height);
        let top_bar_max_y = extension_bar
            .map(|bar| bar.y + bar.height)
            .unwrap_or(fraction_bar.y + fraction_bar.height);
        let (radical_bar_left_local, radical_bar_right_local) = path_x_range_in_y_band(
            radical_path,
            top_bar_min_y - radical_y,
            top_bar_max_y - radical_y,
        )
        .expect("radical path must intersect top bar y-band");
        let radical_bar_left = radical_x + radical_bar_left_local;
        let radical_bar_right = radical_x + radical_bar_right_local;

        let extension_left = extension_bar.map(|bar| bar.x).unwrap_or(f32::INFINITY);
        let extension_right = extension_bar
            .map(|bar| bar.x + bar.width)
            .unwrap_or(f32::NEG_INFINITY);
        let covered_left = radical_bar_left.min(extension_left);
        let covered_right = radical_bar_right.max(extension_right);

        let fraction_right = fraction_bar.x + fraction_bar.width;
        let epsilon = 0.01_f32;
        assert!(
            covered_left <= fraction_bar.x + epsilon,
            "sqrt top bar must start no later than fraction rule: covered_left={covered_left}, fraction_bar={fraction_bar:?}, radical_bar_left={radical_bar_left}, extension={extension_bar:?}"
        );
        assert!(
            covered_right + epsilon >= fraction_right,
            "sqrt top bar must cover fraction rule right edge: covered_right={covered_right}, fraction_bar={fraction_bar:?}, radical_bar_right={radical_bar_right}, extension={extension_bar:?}"
        );

        let left_overhang = fraction_bar.x - covered_left;
        let max_left_overhang = FONT_SIZE * 0.30;
        assert!(
            left_overhang <= max_left_overhang,
            "sqrt top bar left overhang too large for fraction radicand: left_overhang={left_overhang}, max={max_left_overhang}, covered_left={covered_left}, fraction_bar={fraction_bar:?}, extension={extension_bar:?}"
        );
    }

    #[test]
    fn sqrt_fraction_bar_stays_right_of_radical_outline() {
        let layout = layout_latex_for_test(r"\sqrt{\frac{a+b}{c+d}}", 30.0);
        let mut rects = extract_rect_commands(&layout);
        assert!(
            rects.len() >= 2,
            "sqrt fraction layout should have overbar and fraction bar"
        );
        rects.sort_by(|a, b| a.y.total_cmp(&b.y));
        let fraction_bar = rects[rects.len() - 1];
        let (radical_path, radical_x, radical_y) = radical_path_command(&layout);
        let radical_right = path_rightmost_x_in_y_band(
            radical_path,
            fraction_bar.y - radical_y,
            fraction_bar.y + fraction_bar.height - radical_y,
        )
        .expect("radical path must intersect fraction bar y-band");
        let right_edge = radical_x + radical_right;
        assert!(
            fraction_bar.x >= right_edge,
            "fraction bar overlaps radical outline: fraction_x={}, radical_right={right_edge}, fraction_bar={fraction_bar:?}, radical_y={radical_y}",
            fraction_bar.x
        );
    }

    #[test]
    fn offscreen_sqrt_join_has_visible_connected_pixels() {
        const FORMULA: &str = r"\sqrt{\frac{a+b}{c+d}}";
        const FONT_SIZE: f32 = 72.0;
        const PADDING: f32 = 16.0;
        let layout = layout_latex_for_test(FORMULA, FONT_SIZE);
        let mut rects = extract_rect_commands(&layout);
        rects.sort_by(|a, b| a.y.total_cmp(&b.y));
        let overbar = rects
            .first()
            .copied()
            .expect("sqrt formula should include overbar rect");
        let baseline_y = PADDING + layout.ascent;
        let sample_x = (PADDING + overbar.x).round() as i32;
        let sample_y = (baseline_y + overbar.y + overbar.height * 0.5).round() as i32;

        let output = render_latex_offscreen(FORMULA, FONT_SIZE, PADDING, None);
        assert!(
            has_foreground_in_window(
                &output,
                sample_x - 2,
                sample_x + 2,
                sample_y - 2,
                sample_y + 2
            ),
            "sqrt overbar start should paint visible pixels near join: sample=({sample_x},{sample_y})"
        );
        assert!(
            has_foreground_in_window(
                &output,
                sample_x - 7,
                sample_x - 1,
                sample_y - 2,
                sample_y + 2
            ),
            "sqrt radical stroke should be visible immediately left of overbar join: sample=({sample_x},{sample_y})"
        );
    }

    #[test]
    fn decodes_numeric_entities() {
        let decoded = decode_entities("&#x03B1; + &#960;").expect("entity decode");
        assert_eq!(decoded, "α + π");
    }

    #[test]
    fn renders_math_scene_offscreen_via_gpu_surface() {
        let output = render_latex_offscreen(r"\frac{1+\sqrt{x}}{1+x^2}", 28.0, 6.0, None);
        let width = output.width;
        let height = output.height;

        assert_eq!(output.width, width);
        assert_eq!(output.height, height);
        assert_eq!(output.rgba8.len(), (width * height * 4) as usize);

        let opaque_pixels = output
            .rgba8
            .chunks_exact(4)
            .filter(|px| is_foreground_pixel(px))
            .count();
        assert!(
            opaque_pixels > 0,
            "math offscreen output must contain visible pixels"
        );
        let first = &output.rgba8[..4];
        let has_variation = output.rgba8.chunks_exact(4).any(|px| px != first);
        assert!(
            has_variation,
            "math offscreen output must not be a single flat color"
        );

        let png = output
            .to_png()
            .expect("math offscreen PNG encoding should succeed");
        const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
        assert!(png.len() >= PNG_SIGNATURE.len());
        assert_eq!(&png[..PNG_SIGNATURE.len()], &PNG_SIGNATURE);

        if let Ok(path) = std::env::var("WATERUI_MATH_OFFSCREEN_OUT") {
            let out_path = std::path::PathBuf::from(path);
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)
                    .expect("math offscreen output dir must be creatable");
            }
            output
                .save_png(&out_path)
                .expect("math offscreen png should be writable");
            assert!(
                out_path.exists(),
                "math offscreen png file should exist at {out_path:?}"
            );
        }
    }

    #[test]
    fn renders_common_llm_formulas_without_clipping() {
        const CANVAS_WIDTH: u32 = 280;
        const CANVAS_HEIGHT: u32 = 164;
        const FONT_SIZE: f32 = 28.0;
        const PADDING: f32 = 8.0;

        let gallery_dir = std::env::var("WATERUI_MATH_GALLERY_DIR")
            .ok()
            .map(PathBuf::from);
        if let Some(dir) = &gallery_dir {
            std::fs::create_dir_all(dir).expect("math gallery output directory must be creatable");
        }
        let mut gallery_index = String::new();

        for (index, formula) in COMMON_LLM_LATEX.iter().enumerate() {
            let output = render_latex_offscreen(
                formula,
                FONT_SIZE,
                PADDING,
                Some((CANVAS_WIDTH, CANVAS_HEIGHT)),
            );
            let Some(bounds) = foreground_bounds(&output) else {
                panic!("formula rendered no visible foreground pixels: '{formula}'");
            };
            assert!(
                bounds.min_x > 0
                    && bounds.min_y > 0
                    && bounds.max_x + 1 < output.width
                    && bounds.max_y + 1 < output.height,
                "formula likely clipped at image edge: '{formula}', bounds={bounds:?}, canvas={}x{}",
                output.width,
                output.height
            );

            let foreground_pixels = output
                .rgba8
                .chunks_exact(4)
                .filter(|px| is_foreground_pixel(px))
                .count();
            assert!(
                foreground_pixels >= 40,
                "formula foreground area unexpectedly small: '{formula}', foreground_pixels={foreground_pixels}"
            );

            if let Some(dir) = &gallery_dir {
                let png_path = dir.join(format!("formula_{index:02}.png"));
                output
                    .save_png(&png_path)
                    .expect("formula gallery PNG should be writable");
                assert!(
                    png_path.exists(),
                    "formula gallery PNG should exist: {png_path:?}"
                );
                gallery_index.push_str(&format!("formula_{index:02}.png: {formula}\n"));
            }
        }

        if let Some(dir) = &gallery_dir {
            let index_path = dir.join("index.txt");
            std::fs::write(&index_path, gallery_index)
                .expect("formula gallery index should be writable");
            assert!(
                index_path.exists(),
                "formula gallery index should exist: {index_path:?}"
            );
        }
    }
}
