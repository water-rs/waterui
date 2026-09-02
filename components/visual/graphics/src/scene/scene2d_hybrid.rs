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

use std::collections::HashMap;

use kurbo::{Affine, BezPath, Rect, Stroke};
use peniko::{BlendMode, Brush, Fill, ImageBrush, ImageData, StyleRef, WeakBlob};
use vello_common::paint::{Image, ImageId, ImageSource};

use crate::scene2d::{GlyphRun, Scene2D};

/// The hybrid renderer and the resources it draws with.
///
/// They are created together and used together, so they are kept together: the
/// renderer owns the atlas texture, and the resources own what has been placed
/// in it.
#[derive(Debug)]
pub struct HybridRenderer {
    /// The renderer itself.
    pub renderer: vello_hybrid::Renderer,
    /// Its atlas and buffer resources.
    pub resources: vello_hybrid::Resources,
    /// The images this renderer has already put in its atlas.
    pub images: HybridImageAtlas,
}

/// One image this renderer has uploaded into its atlas.
#[derive(Debug)]
struct AtlasEntry {
    /// Handle naming the atlas allocation.
    id: ImageId,
    /// Whether the uploaded pixels have any pixel that is not fully opaque.
    ///
    /// Computed once, when the pixels are converted, because the answer is a
    /// property of the pixels rather than of the draw that samples them.
    may_have_transparency: bool,
    /// The pixels this entry was uploaded from, held weakly.
    ///
    /// A `Blob` is the identity of an image's pixels, so the moment the last
    /// owner drops it nothing can ask for this image again, and its atlas space
    /// is free for the next image that needs room.
    pixels: WeakBlob<u8>,
}

/// The images one hybrid renderer has uploaded into its atlas.
///
/// The atlas is a texture the renderer owns, so what is in it belongs to the
/// renderer too, and not to any one scene: an image drawn on every frame is
/// uploaded once and sampled from the atlas on every frame after that. Entries
/// are keyed by the identity of the pixels they were uploaded from, which is
/// how the same image drawn from two different scenes resolves to one
/// allocation.
#[derive(Debug, Default)]
pub struct HybridImageAtlas {
    uploaded: HashMap<u64, AtlasEntry>,
}

/// The device handles that putting pixels into the atlas needs.
///
/// The atlas is a texture, so filling it is device work rather than scene
/// recording: an upload is a queued texture write, and growing the atlas is a
/// copy recorded into `encoder` — the same encoder the frame is rendered with,
/// so the write lands before the pass that samples it.
pub struct HybridUpload<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    encoder: &'a mut wgpu::CommandEncoder,
}

impl core::fmt::Debug for HybridUpload<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("HybridUpload")
            .finish_non_exhaustive()
    }
}

impl<'a> HybridUpload<'a> {
    /// Names the device, queue and encoder an upload is recorded against.
    #[must_use]
    pub const fn new(
        device: &'a wgpu::Device,
        queue: &'a wgpu::Queue,
        encoder: &'a mut wgpu::CommandEncoder,
    ) -> Self {
        Self {
            device,
            queue,
            encoder,
        }
    }
}

impl HybridImageAtlas {
    /// The atlas handle for `data`, uploading its pixels the first time this
    /// renderer is asked for them.
    ///
    /// # Panics
    ///
    /// Panics when the image is larger than an atlas can hold, or when the
    /// atlas has no room left for it.
    fn source_for(
        &mut self,
        renderer: &mut vello_hybrid::Renderer,
        resources: &mut vello_hybrid::Resources,
        upload: &mut HybridUpload<'_>,
        data: &ImageData,
    ) -> ImageSource {
        let key = data.data.id();
        if let Some(entry) = self.uploaded.get(&key) {
            return ImageSource::opaque_id_with_transparency_hint(
                entry.id,
                entry.may_have_transparency,
            );
        }

        self.release_dropped(renderer, resources, upload);

        // The conversion to premultiplied atlas pixels is the same one the
        // engine would do for itself; only its result travels differently,
        // because this renderer samples an atlas handle rather than a pixmap
        // carried along with the scene.
        let ImageSource::Pixmap(pixels) = ImageSource::from_peniko_image_data(data) else {
            panic!("`ImageSource::from_peniko_image_data` produces decoded pixels");
        };
        let may_have_transparency = pixels.may_have_transparency();
        let id = renderer.upload_image(
            resources,
            upload.device,
            upload.queue,
            upload.encoder,
            &pixels,
        );
        self.uploaded.insert(
            key,
            AtlasEntry {
                id,
                may_have_transparency,
                pixels: data.data.downgrade(),
            },
        );
        ImageSource::opaque_id_with_transparency_hint(id, may_have_transparency)
    }

    /// Frees the atlas space held by images whose pixels the application has
    /// released.
    ///
    /// Clearing an allocation is a render pass, and a queued texture write runs
    /// before every command buffer submitted alongside it — so a clear recorded
    /// into the frame's encoder would execute *after* the upload that follows
    /// it here and wipe the image just placed in the reclaimed space. The
    /// clears therefore go in an encoder of their own and are submitted before
    /// the upload is queued, which puts them in an earlier submission and back
    /// in the order they were asked for.
    fn release_dropped(
        &mut self,
        renderer: &mut vello_hybrid::Renderer,
        resources: &mut vello_hybrid::Resources,
        upload: &HybridUpload<'_>,
    ) {
        if self
            .uploaded
            .values()
            .all(|entry| entry.pixels.upgrade().is_some())
        {
            return;
        }

        let mut encoder = upload
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("hybrid scene image atlas release"),
            });
        self.uploaded.retain(|_, entry| {
            if entry.pixels.upgrade().is_some() {
                return true;
            }
            renderer.destroy_image(resources, &mut encoder, entry.id);
            false
        });
        upload.queue.submit([encoder.finish()]);
    }
}

/// Wraps a hybrid scene as an engine-independent [`Scene2D`].
pub struct HybridScene2D<'a> {
    scene: &'a mut vello_hybrid::Scene,
    // Glyphs rasterize into an atlas, and images upload into another, both of
    // which outlive any one scene. Drawing either therefore needs the renderer
    // and its resources, not just the scene.
    renderer: &'a mut HybridRenderer,
    upload: HybridUpload<'a>,
}

impl core::fmt::Debug for HybridScene2D<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("HybridScene2D")
            .finish_non_exhaustive()
    }
}

impl<'a> HybridScene2D<'a> {
    /// Wraps a mutable hybrid scene, the renderer it draws with, and the device
    /// handles that renderer's atlases are filled through.
    #[must_use]
    pub const fn new(
        scene: &'a mut vello_hybrid::Scene,
        renderer: &'a mut HybridRenderer,
        upload: HybridUpload<'a>,
    ) -> Self {
        Self {
            scene,
            renderer,
            upload,
        }
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
            .glyph_run(&mut self.renderer.resources, run.font)
            .font_size(run.font_size)
            .normalized_coords(run.normalized_coords);
        if stroked {
            builder.stroke_glyphs(glyphs);
        } else {
            builder.fill_glyphs(glyphs);
        }
    }

    fn draw_image(&mut self, image: &ImageBrush, transform: Affine) {
        let Self {
            scene,
            renderer,
            upload,
        } = self;
        let HybridRenderer {
            renderer,
            resources,
            images,
        } = &mut **renderer;
        let source = images.source_for(renderer, resources, upload, &image.image);

        // Classic fills the image's own pixel rectangle with the image as its
        // brush, which puts the top-left pixel at the transform's origin and
        // one image pixel on one unit. The paint transform is reset rather than
        // left alone so the image samples in that same rectangle's space,
        // matching what an untransformed brush does over there.
        scene.set_transform(transform);
        scene.set_fill_rule(Fill::NonZero);
        scene.reset_paint_transform();
        scene.set_paint(Image {
            image: source,
            sampler: image.sampler,
        });
        scene.fill_rect(&Rect::new(
            0.0,
            0.0,
            f64::from(image.image.width),
            f64::from(image.image.height),
        ));
    }

    fn reset(&mut self) {
        self.scene.reset();
    }
}
