use kurbo::{Affine, BezPath, Stroke};
use peniko::{BlendMode, Brush, Fill, ImageBrush};

use crate::scene2d::{GlyphRun, Scene2D};

/// Vello-backed `Scene2D` implementation.
pub struct VelloScene2D<'a> {
    scene: &'a mut vello::Scene,
}

impl core::fmt::Debug for VelloScene2D<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VelloScene2D").finish_non_exhaustive()
    }
}

impl<'a> VelloScene2D<'a> {
    /// Wraps a mutable Vello scene.
    #[must_use]
    pub const fn new(scene: &'a mut vello::Scene) -> Self {
        Self { scene }
    }

    /// Appends an existing Vello scene directly.
    pub fn append(&mut self, scene: &vello::Scene, transform: Option<Affine>) {
        self.scene.append(scene, transform);
    }

    /// Returns mutable access to the underlying Vello scene.
    #[must_use]
    pub const fn scene_mut(&mut self) -> &mut vello::Scene {
        self.scene
    }
}

impl Scene2D for VelloScene2D<'_> {
    fn fill(
        &mut self,
        fill: Fill,
        transform: Affine,
        brush: &Brush,
        brush_transform: Option<Affine>,
        shape: &BezPath,
    ) {
        self.scene
            .fill(fill, transform, brush, brush_transform, shape);
    }

    fn stroke(
        &mut self,
        stroke: &Stroke,
        transform: Affine,
        brush: &Brush,
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
        self.scene.push_clip_layer(fill, transform, clip);
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

    fn append_vello_scene(&mut self, scene: &vello::Scene, transform: Option<Affine>) {
        self.scene.append(scene, transform);
    }

    fn encode_vello(&mut self, encode: &mut dyn FnMut(&mut vello::Scene)) {
        encode(self.scene);
    }

    fn reset(&mut self) {
        self.scene.reset();
    }
}
