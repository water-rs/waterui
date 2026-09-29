//! The legacy Vello implementation of the recording boundary — the only file
//! outside `crate::engine::vello_backend` whose scene internals may name
//! `vello::Scene`. Everything here is deleted at cutover.

use crate::engine::{Brush, DrawContext};
use kurbo::{
    Affine, BezPath, Circle, Line, Point, Rect, RoundedRect, RoundedRectRadii, Shape, Vec2,
};
use peniko::{BlendMode, Fill, ImageBrush};
use waterui_graphics::{GlyphRun, Scene2D};

/// The opaque recording object drawing code builds. Internally a legacy
/// `vello::Scene`; nothing about that reaches the drawing-facing API.
///
/// The methods on this type are the fixed boundary API. The impls are plain
/// delegation to the equivalent `vello::Scene` call — including the
/// transform/style force-flag re-arms the Vello encoding needs at clip
/// boundaries (water-rs/hydrolysis#250), preserved verbatim until the engine
/// switch.
#[derive(Clone)]
pub struct Recording {
    scene: vello::Scene,
}

impl Default for Recording {
    fn default() -> Self {
        Self::new()
    }
}

impl Recording {
    /// Creates an empty recording.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            scene: vello::Scene::new(),
        }
    }

    /// Clears all recorded commands.
    pub(crate) fn reset(&mut self) {
        self.scene.reset();
    }

    /// Whether the recording encodes any visible content.
    ///
    /// `Encoding::is_empty` only checks the path stream; glyph runs are
    /// deferred resources that resolve to paths at render time, so a recording
    /// containing only text would otherwise read as empty and be dropped by
    /// the compositor.
    pub(crate) fn is_empty(&self) -> bool {
        let encoding = self.scene.encoding();
        encoding.is_empty() && encoding.resources.glyph_runs.is_empty()
    }

    /// Fills `shape` under `transform` with `brush`.
    pub(crate) fn fill<S: Shape>(
        &mut self,
        rule: Fill,
        transform: Affine,
        brush: &peniko::Brush,
        brush_transform: Option<Affine>,
        shape: &S,
    ) {
        self.scene
            .fill(rule, transform, brush, brush_transform, shape);
    }

    /// Strokes `shape` under `transform` with `brush`.
    pub(crate) fn stroke<S: Shape>(
        &mut self,
        stroke: &kurbo::Stroke,
        transform: Affine,
        brush: &peniko::Brush,
        brush_transform: Option<Affine>,
        shape: &S,
    ) {
        self.scene
            .stroke(stroke, transform, brush, brush_transform, shape);
    }

    /// Draws an image under `transform`.
    pub(crate) fn image(&mut self, image: &ImageBrush, transform: Affine) {
        self.scene.draw_image(image, transform);
    }

    /// Draws a run of shaped glyphs.
    pub(crate) fn glyphs(&mut self, run: GlyphRun<'_>) {
        self.scene
            .draw_glyphs(run.font)
            .brush(run.brush)
            .brush_alpha(run.brush_alpha)
            .transform(run.transform)
            .font_size(run.font_size)
            .normalized_coords(run.normalized_coords)
            .draw(
                run.style,
                run.glyphs.iter().map(|glyph| vello::Glyph {
                    id: glyph.id,
                    x: glyph.x,
                    y: glyph.y,
                }),
            );
    }

    /// Draws a blurred rounded rect shadow under `transform`.
    pub(crate) fn blurred_rounded_rect(
        &mut self,
        transform: Affine,
        rect: Rect,
        color: peniko::Color,
        corner_radius: f64,
        sigma: f64,
    ) {
        self.scene
            .draw_blurred_rounded_rect(transform, rect, color, corner_radius, sigma);
    }

    /// Pushes a clip-only scope.
    pub(crate) fn push_clip<S: Shape>(&mut self, rule: Fill, transform: Affine, shape: &S) {
        self.scene.push_clip_layer(rule, transform, shape);
    }

    /// Pushes a compositing scope with blend mode, opacity and clip.
    pub(crate) fn push_group<S: Shape>(
        &mut self,
        rule: Fill,
        blend: BlendMode,
        opacity: f32,
        clip_transform: Affine,
        clip: &S,
    ) {
        self.scene
            .push_layer(rule, blend, opacity, clip_transform, clip);
    }

    /// Closes the current scope, then re-arms the encoding's transform/style
    /// force flags.
    ///
    /// `pop_layer`/`encode_end_clip` emits a dummy `PATH` with no transform or
    /// style — nothing re-arms the encoder's force flags at the boundary, so a
    /// path drawn next dedups its transform against the encoding's last entry.
    /// Once the resolver splices a glyph run's transform entries into the
    /// packed stream ahead of that path, the path's `trans_ix` lands on the
    /// run's paint transform instead of its own (water-rs/hydrolysis#250).
    /// Re-arming the force flags keeps every post-clip path carrying its own
    /// transform.
    pub(crate) fn pop_scope(&mut self) {
        self.scene.pop_layer();
        self.scene.encoding_mut().force_next_transform_and_style();
    }

    /// Appends `other` under `placement`, then re-arms the encoding's
    /// transform/style force flags: `Encoding::append` copies the child's
    /// `flags` verbatim, so a force pending in this encoding is silently
    /// dropped for the next encode — the same clip-boundary hole
    /// [`Self::pop_scope`] covers (water-rs/hydrolysis#250).
    pub(crate) fn append(&mut self, other: &Recording, placement: Affine) {
        self.scene.append(&other.scene, Some(placement));
        self.scene.encoding_mut().force_next_transform_and_style();
    }
}

/// Legacy plumbing — submission-side and tracking access, not part of the
/// drawing-facing API. Everything below is consumed only by the compositor's
/// submission boundary, `crate::engine::vello_backend`, or tests already
/// quarantined in the boundary baseline.
impl Recording {
    /// Scene-layer scopes still open, for the tracked-stack invariant the
    /// compositor asserts at flush.
    pub(crate) fn open_clip_count(&self) -> u32 {
        self.scene.encoding().n_open_clips
    }

    /// Read access for the legacy submission path: the compositor renders a
    /// recording's encoding through `vello::Renderer`, and this is the only
    /// sanctioned way to reach it. Drawing code must never use it.
    pub(crate) fn legacy_scene(&self) -> &vello::Scene {
        &self.scene
    }
}

/// The existing `Scene2D` contract, preserved unchanged on `Recording` so
/// current WaterUI and hydrolysis-m3 call sites keep compiling. Semantics
/// match the previous `VelloScene2D` exactly — including a plain `pop_layer`
/// with no force-flag re-arm.
impl Scene2D for Recording {
    fn fill(
        &mut self,
        fill: Fill,
        transform: Affine,
        brush: &peniko::Brush,
        brush_transform: Option<Affine>,
        shape: &BezPath,
    ) {
        self.scene
            .fill(fill, transform, brush, brush_transform, shape);
    }

    fn stroke(
        &mut self,
        stroke: &kurbo::Stroke,
        transform: Affine,
        brush: &peniko::Brush,
        brush_transform: Option<Affine>,
        shape: &BezPath,
    ) {
        self.scene
            .stroke(stroke, transform, brush, brush_transform, shape);
    }

    fn push_layer(
        &mut self,
        fill: Fill,
        blend: BlendMode,
        alpha: f32,
        transform: Affine,
        clip: &BezPath,
    ) {
        self.scene.push_layer(fill, blend, alpha, transform, clip);
    }

    fn push_clip_layer(&mut self, fill: Fill, transform: Affine, clip: &BezPath) {
        self.push_clip(fill, transform, clip);
    }

    fn pop_layer(&mut self) {
        self.scene.pop_layer();
    }

    fn draw_image(&mut self, image: &ImageBrush, transform: Affine) {
        self.scene.draw_image(image, transform);
    }

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_>) {
        self.scene
            .draw_glyphs(run.font)
            .brush(run.brush)
            .brush_alpha(run.brush_alpha)
            .transform(run.transform)
            .font_size(run.font_size)
            .normalized_coords(run.normalized_coords)
            .draw(
                run.style,
                run.glyphs.iter().map(|glyph| vello::Glyph {
                    id: glyph.id,
                    x: glyph.x,
                    y: glyph.y,
                }),
            );
    }

    fn reset(&mut self) {
        self.scene.reset();
    }
}

/// The existing `DrawContext` adapter: the WaterUI theme-drawing interface,
/// recorded into [`Recording`]. Moved here unchanged from
/// `engine::vello_backend` — deleted with the boundary at cutover.
pub struct VelloDrawContext<'a> {
    scene: &'a mut Recording,
    transform_stack: Vec<Affine>,
}

impl<'a> VelloDrawContext<'a> {
    pub(crate) fn with_root_transform(scene: &'a mut Recording, transform: Affine) -> Self {
        Self {
            scene,
            transform_stack: vec![Affine::IDENTITY, transform],
        }
    }

    fn transform(&self) -> Affine {
        *self
            .transform_stack
            .last()
            .expect("vello draw context transform stack is empty")
    }

    fn fill_shape(&mut self, shape: &impl Shape, brush: &Brush) {
        let brush = match brush {
            Brush::Solid(color) => peniko::Brush::Solid(*color),
            Brush::Gradient(gradient) => peniko::Brush::Gradient(gradient.clone()),
        };
        self.scene
            .fill(Fill::NonZero, self.transform(), &brush, None, shape);
    }

    fn stroke_shape(&mut self, shape: &impl Shape, brush: &Brush, width: f64) {
        let stroke = kurbo::Stroke::new(width);
        let brush = match brush {
            Brush::Solid(color) => peniko::Brush::Solid(*color),
            Brush::Gradient(gradient) => peniko::Brush::Gradient(gradient.clone()),
        };
        self.scene
            .stroke(&stroke, self.transform(), &brush, None, shape);
    }
}

impl DrawContext for VelloDrawContext<'_> {
    fn fill_rect(&mut self, rect: Rect, brush: &Brush) {
        self.fill_shape(&rect, brush);
    }

    fn fill_rounded_rect(&mut self, rect: Rect, radii: RoundedRectRadii, brush: &Brush) {
        let rounded = RoundedRect::from_rect(rect, radii);
        self.fill_shape(&rounded, brush);
    }

    fn stroke_rect(&mut self, rect: Rect, brush: &Brush, width: f64) {
        self.stroke_shape(&rect, brush, width);
    }

    fn stroke_rounded_rect(
        &mut self,
        rect: Rect,
        radii: RoundedRectRadii,
        brush: &Brush,
        width: f64,
    ) {
        let rounded = RoundedRect::from_rect(rect, radii);
        self.stroke_shape(&rounded, brush, width);
    }

    fn stroke_line(&mut self, from: Point, to: Point, brush: &Brush, width: f64) {
        let line = Line::new(from, to);
        self.stroke_shape(&line, brush, width);
    }

    fn stroke_circle(&mut self, center: Point, radius: f64, brush: &Brush, width: f64) {
        let circle = Circle::new(center, radius);
        self.stroke_shape(&circle, brush, width);
    }

    fn fill_circle(&mut self, center: Point, radius: f64, brush: &Brush) {
        let circle = Circle::new(center, radius);
        self.fill_shape(&circle, brush);
    }

    fn fill_path(&mut self, path: &BezPath, brush: &Brush) {
        self.fill_shape(path, brush);
    }

    fn stroke_path(&mut self, path: &BezPath, brush: &Brush, width: f64) {
        self.stroke_shape(path, brush, width);
    }

    fn draw_shadow(
        &mut self,
        rect: Rect,
        radii: RoundedRectRadii,
        offset: Vec2,
        blur: f64,
        color: peniko::Color,
    ) {
        let radius = radii
            .as_single_radius()
            .expect("vello blurred shadows require uniform corner radii");
        self.scene.blurred_rounded_rect(
            self.transform(),
            rect + offset,
            color,
            radius,
            blur.max(0.0),
        );
    }

    fn push_layer(&mut self, alpha: f32, clip: Option<&Rect>) {
        let clip = clip
            .copied()
            .unwrap_or(Rect::new(-1.0e9, -1.0e9, 1.0e9, 1.0e9));
        self.scene.push_group(
            Fill::NonZero,
            BlendMode::default(),
            alpha,
            self.transform(),
            &clip,
        );
    }

    fn push_rounded_layer(&mut self, alpha: f32, clip: Rect, radii: RoundedRectRadii) {
        let clip = RoundedRect::from_rect(clip, radii);
        self.scene.push_group(
            Fill::NonZero,
            BlendMode::default(),
            alpha,
            self.transform(),
            &clip,
        );
    }

    fn pop_layer(&mut self) {
        self.scene.pop_scope();
    }

    fn push_transform(&mut self, affine: Affine) {
        let current = self.transform();
        self.transform_stack.push(current * affine);
    }

    fn pop_transform(&mut self) {
        assert!(
            self.transform_stack.len() > 1,
            "vello draw context transform stack underflow"
        );
        self.transform_stack.pop();
    }
}
