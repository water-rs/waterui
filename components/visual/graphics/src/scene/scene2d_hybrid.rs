//! [`Scene2D`](crate::Scene2D) over the CPU/GPU split renderer.
//!
//! Vello classic rasterizes through a compute pipeline that needs indirect
//! execution. Some devices have no such thing — the iOS Simulator's Metal is
//! the one that matters here — and on those, drawing anything scene-backed
//! aborts the process. This engine processes paths on the CPU and rasterizes on
//! the GPU, so it asks for nothing the simulator cannot do, without falling all
//! the way back to a CPU rasterizer.
//!
//! The engines differ in how they take drawing state: classic takes the
//! transform and brush with each call, this one is set first and drawn after.
//! That is the whole of the translation below.

use kurbo::{Affine, BezPath, Stroke};
use peniko::{BlendMode, Brush, Fill, ImageBrush, StyleRef};

use crate::scene2d::{GlyphRun, Scene2D};

/// Wraps a hybrid scene as an engine-independent [`Scene2D`].
pub struct HybridScene2D<'a> {
    scene: &'a mut vello_hybrid::Scene,
    // Glyphs are rasterized into an atlas that outlives any one scene, so
    // drawing text needs the renderer's resources and not just the scene.
    resources: &'a mut vello_hybrid::Resources,
}

impl core::fmt::Debug for HybridScene2D<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("HybridScene2D")
            .finish_non_exhaustive()
    }
}

impl<'a> HybridScene2D<'a> {
    /// Wraps a mutable hybrid scene and the renderer resources it draws with.
    #[must_use]
    pub const fn new(
        scene: &'a mut vello_hybrid::Scene,
        resources: &'a mut vello_hybrid::Resources,
    ) -> Self {
        Self { scene, resources }
    }
}

/// Applies a brush as this engine's paint, if it is one this engine draws.
///
/// The two engines spell an image brush differently — this one names its own
/// image source — so a brush is translated rather than handed over. Solid
/// colours and gradients cover everything `WaterUI` draws through a scene; an
/// image is drawn by [`Scene2D::draw_image`] rather than by being set as paint.
fn set_paint(scene: &mut vello_hybrid::Scene, brush: &Brush) -> bool {
    match brush {
        Brush::Solid(color) => {
            scene.set_paint(*color);
            true
        }
        Brush::Gradient(gradient) => {
            scene.set_paint(gradient.clone());
            true
        }
        Brush::Image(_) => false,
    }
}

/// Positions the paint independently of the shape, or back on the shape when
/// there is no separate brush transform.
fn set_paint_transform(scene: &mut vello_hybrid::Scene, brush_transform: Option<Affine>) {
    match brush_transform {
        Some(transform) => scene.set_paint_transform(transform),
        None => scene.reset_paint_transform(),
    }
}

impl Scene2D for HybridScene2D<'_> {
    fn fill(
        &mut self,
        fill: Fill,
        transform: Affine,
        brush: &Brush,
        brush_transform: Option<Affine>,
        shape: &BezPath,
    ) {
        self.scene.set_transform(transform);
        self.scene.set_fill_rule(fill);
        set_paint_transform(self.scene, brush_transform);
        if set_paint(self.scene, brush) {
            self.scene.fill_path(shape);
        }
    }

    fn stroke(
        &mut self,
        stroke: &Stroke,
        transform: Affine,
        brush: &Brush,
        brush_transform: Option<Affine>,
        shape: &BezPath,
    ) {
        self.scene.set_transform(transform);
        self.scene.set_stroke(stroke.clone());
        set_paint_transform(self.scene, brush_transform);
        if set_paint(self.scene, brush) {
            self.scene.stroke_path(shape);
        }
    }

    fn push_layer(
        &mut self,
        _fill: Fill,
        blend: BlendMode,
        alpha: f32,
        transform: Affine,
        clip: &BezPath,
    ) {
        self.scene.set_transform(transform);
        self.scene
            .push_layer(Some(clip), Some(blend), Some(alpha), None, None);
    }

    fn push_clip_layer(&mut self, _fill: Fill, transform: Affine, clip: &BezPath) {
        self.scene.set_transform(transform);
        self.scene.push_clip_layer(clip);
    }

    fn pop_layer(&mut self) {
        self.scene.pop_layer();
    }

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_>) {
        // Drawing state is set on the scene and read when the run is drawn, so
        // all of it has to be in place before the builder borrows the scene.
        self.scene.set_transform(run.transform);
        let paint_applied = match run.brush {
            Brush::Solid(color) => {
                self.scene.set_paint(color.multiply_alpha(run.brush_alpha));
                true
            }
            _ => set_paint(self.scene, run.brush),
        };
        if !paint_applied {
            return;
        }
        let stroked = match run.style {
            StyleRef::Fill(fill) => {
                self.scene.set_fill_rule(fill);
                false
            }
            StyleRef::Stroke(stroke) => {
                self.scene.set_stroke(stroke.clone());
                true
            }
        };

        let glyphs = run.glyphs.iter().map(|glyph| glifo::Glyph {
            id: glyph.id,
            x: glyph.x,
            y: glyph.y,
        });

        let builder = self
            .scene
            .glyph_run(self.resources, run.font)
            .font_size(run.font_size)
            .normalized_coords(run.normalized_coords);
        if stroked {
            builder.stroke_glyphs(glyphs);
        } else {
            builder.fill_glyphs(glyphs);
        }
    }

    fn draw_image(&mut self, image: &ImageBrush, _transform: Affine) {
        // Images in a scene need this engine's own image source, uploaded into
        // its atlas — not the decoded pixels a `peniko::ImageBrush` carries.
        // Nothing WaterUI draws through a scene uses one yet: photos and video
        // are their own surfaces, and an SVG's raster images are not drawn
        // either. Left unimplemented rather than silently drawing nothing, so
        // the first caller that needs it says so.
        let _ = image;
        unimplemented!("image brushes in a hybrid scene are not implemented yet");
    }

    fn reset(&mut self) {
        self.scene.reset();
    }
}
