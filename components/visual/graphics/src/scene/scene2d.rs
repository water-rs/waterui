use kurbo::{Affine, BezPath, Stroke};
use peniko::{BlendMode, Brush, Fill, FontData, ImageBrush, StyleRef};

/// One positioned glyph within a run.
///
/// Offsets are relative to the run's transform, as they come out of shaping.
#[derive(Debug, Clone, Copy)]
pub struct Glyph {
    /// Glyph identifier within its font.
    pub id: u32,
    /// X offset within the run.
    pub x: f32,
    /// Y offset within the run.
    pub y: f32,
}

/// A run of glyphs from one font that share every drawing attribute.
///
/// Text is the one thing a scene draws that is not a path, so it is the one
/// thing that cannot be expressed through [`Scene2D::fill`]: each engine turns
/// glyph ids into outlines its own way, with its own caches.
#[derive(Debug)]
pub struct GlyphRun<'a> {
    /// The font these glyphs are indexed in.
    pub font: &'a FontData,
    /// Em size in pixels.
    pub font_size: f32,
    /// Variable-font axis positions, normalized, as shaping produced them.
    pub normalized_coords: &'a [i16],
    /// Transform applied to the whole run.
    pub transform: Affine,
    /// Paint for the glyphs, and the "foreground colour" for colour fonts.
    pub brush: &'a Brush,
    /// Extra alpha multiplier applied to `brush`.
    pub brush_alpha: f32,
    /// Whether the glyphs are filled or stroked.
    pub style: StyleRef<'a>,
    /// The glyphs, in run order.
    pub glyphs: &'a [Glyph],
}

/// Rendering-engine-independent 2D scene builder interface.
pub trait Scene2D {
    /// Fills a shape with the given brush and transform.
    ///
    /// `brush_transform` is applied to the brush on top of `transform`, which is
    /// what positions a gradient independently of the shape it paints.
    fn fill(
        &mut self,
        fill: Fill,
        transform: Affine,
        brush: &Brush,
        brush_transform: Option<Affine>,
        shape: &BezPath,
    );

    /// Strokes a shape with the given brush and transform.
    ///
    /// `brush_transform` is applied to the brush on top of `transform`.
    fn stroke(
        &mut self,
        stroke: &Stroke,
        transform: Affine,
        brush: &Brush,
        brush_transform: Option<Affine>,
        shape: &BezPath,
    );

    /// Pushes a clipped compositing layer.
    fn push_layer(
        &mut self,
        fill: Fill,
        blend: BlendMode,
        alpha: f32,
        transform: Affine,
        clip: &BezPath,
    );

    /// Pushes a clip-only layer.
    fn push_clip_layer(&mut self, fill: Fill, transform: Affine, clip: &BezPath);

    /// Pops the current layer.
    fn pop_layer(&mut self);

    /// Draws an image with transform.
    fn draw_image(&mut self, image: &ImageBrush, transform: Affine);

    /// Draws a run of glyphs.
    fn draw_glyph_run(&mut self, run: &GlyphRun<'_>);

    /// Appends a pre-built Vello scene.
    fn append_vello_scene(&mut self, scene: &vello::Scene, transform: Option<Affine>);

    /// Encodes directly into the backing Vello scene when available.
    ///
    /// Other scene implementations preserve the same behavior by recording into
    /// a temporary scene and appending it after `encode` returns.
    #[doc(hidden)]
    fn encode_vello(&mut self, encode: &mut dyn FnMut(&mut vello::Scene)) {
        let mut scene = vello::Scene::new();
        encode(&mut scene);
        self.append_vello_scene(&scene, None);
    }

    /// Clears all recorded scene commands.
    fn reset(&mut self);
}
