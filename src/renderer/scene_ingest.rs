//! Validation at the point application content hands images to the scene.
//!
//! A malformed `peniko::ImageData` — a `data` blob whose length is not
//! `width * height * bytes_per_pixel(format)` — records into the vello scene
//! without complaint and only fails inside `wgpu`'s `Queue::write_texture`,
//! with a validation message naming neither the image nor the view. Checking
//! the blob here, at the first hydrolysis-owned point, fails fast with the
//! expected and actual byte counts and the format.

use kurbo::{Affine, BezPath, Stroke};
use peniko::{BlendMode, Brush, Fill, ImageBrush, ImageData};
use waterui_graphics::{GlyphRun, Scene2D};

use super::MigrationCounters;

/// Panics unless `image`'s blob is exactly `format.size_in_bytes(width,
/// height)` — the `width * height * bytes_per_pixel` contract wgpu's texture
/// upload enforces far later, and with no pointer back to the offending
/// image.
pub(crate) fn assert_well_formed_image(image: &ImageData) {
    let actual = image.data.len();
    let Some(expected) = image.format.size_in_bytes(image.width, image.height) else {
        panic!(
            "hydrolysis scene ingest: malformed peniko::ImageData — format {:?} at {}x{} \
             overflows the byte-size calculation",
            image.format, image.width, image.height,
        );
    };
    assert!(
        actual == expected,
        "hydrolysis scene ingest: malformed peniko::ImageData — format {:?} at {}x{} needs \
         width*height*bytes_per_pixel = {} bytes, but the blob holds {} bytes; re-encode the \
         image or fix its declared format and dimensions",
        image.format,
        image.width,
        image.height,
        expected,
        actual,
    );
}

/// The same check for an image carried inside a `Brush`, since fills, strokes
/// and glyph-run paints can hold `Brush::Image` exactly like `draw_image`.
/// Each sighting is one image payload registered into the frame, which the
/// migration counters charge the way `draw_image` is.
fn assert_brush_well_formed(brush: &Brush, counters: &mut MigrationCounters) {
    if let Brush::Image(image) = brush {
        assert_well_formed_image(&image.image);
        counters.image_registrations += 1;
    }
}

/// A `Scene2D` that validates every image brush before forwarding to the
/// wrapped builder. This is the boundary where `SceneContent::build_scene`
/// hands the renderer its drawing commands, so it is also where per-frame
/// image registrations are counted.
pub(crate) struct CheckedScene2D<'a> {
    inner: &'a mut dyn Scene2D,
    counters: &'a mut MigrationCounters,
}

impl<'a> CheckedScene2D<'a> {
    pub(crate) fn new(inner: &'a mut dyn Scene2D, counters: &'a mut MigrationCounters) -> Self {
        Self { inner, counters }
    }
}

impl Scene2D for CheckedScene2D<'_> {
    fn fill(
        &mut self,
        fill: Fill,
        transform: Affine,
        brush: &Brush,
        brush_transform: Option<Affine>,
        shape: &BezPath,
    ) {
        assert_brush_well_formed(brush, self.counters);
        self.inner
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
        assert_brush_well_formed(brush, self.counters);
        self.inner
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
        self.inner.push_layer(fill, blend, alpha, transform, clip);
    }

    fn push_clip_layer(&mut self, fill: Fill, transform: Affine, clip: &BezPath) {
        self.inner.push_clip_layer(fill, transform, clip);
    }

    fn pop_layer(&mut self) {
        self.inner.pop_layer();
    }

    fn draw_image(&mut self, image: &ImageBrush, transform: Affine) {
        assert_well_formed_image(&image.image);
        self.counters.image_registrations += 1;
        self.inner.draw_image(image, transform);
    }

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_>) {
        assert_brush_well_formed(run.brush, self.counters);
        self.inner.draw_glyph_run(run);
    }

    fn reset(&mut self) {
        self.inner.reset();
    }
}
