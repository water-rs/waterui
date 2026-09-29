//! Quarantine for the legacy render object and the vello-CPU silhouette
//! rasterizer (water-rs/hydrolysis#205, P2).
//!
//! Everything Vello-specific that is not recording itself lives here:
//! [`Recording`] submission, the renderer's texture/image table, and CPU
//! rasterization of blurred shadow silhouettes. Callers hold the opaque
//! [`LegacyRenderer`] handle — no `vello::Renderer`, `vello::Scene`,
//! `RenderParams` or `AaConfig` type crosses the boundary. Deleted at cutover.

use crate::renderer::Recording;
use std::num::NonZeroUsize;

/// Construction inputs for the legacy renderer, spelled without Vello types.
pub struct LegacyRendererOptions {
    /// Run all stages up to fine rasterization on the CPU (debugging only).
    pub use_cpu: bool,
    /// Pipeline-init thread count; see [`legacy_init_threads`].
    pub num_init_threads: Option<NonZeroUsize>,
    /// The device pipeline cache a persistent store handed out, if any.
    pub pipeline_cache: Option<wgpu::PipelineCache>,
    /// Seed bump-buffer sizes, or `None` to size each render from its
    /// target — grown sizes merge over a seed, so a later shrink never
    /// discards headroom.
    pub buffer_sizes: Option<vello::BumpBufferSizes>,
}

/// Pipeline-init thread count for a [`LegacyRenderer`] on `backend`.
///
/// wgpu-hal's GLES device serialises every shader compile through one context
/// lock with a ~1 s timeout, so handing a parallel init pool on GL deadlocks
/// under CPU load — a worker times out, panics, and poisons the renderer pool.
/// GL therefore gets a single init thread; Vulkan, Metal and DX12 compile
/// pipelines concurrently and keep full parallelism.
pub(crate) fn legacy_init_threads(backend: wgpu::Backend) -> Option<NonZeroUsize> {
    match backend {
        wgpu::Backend::Gl => Some(NonZeroUsize::MIN),
        _ => std::thread::available_parallelism().ok(),
    }
}

/// The opaque legacy render object. Callers submit [`Recording`]s and manage
/// the image table through it; nothing Vello-shaped leaves its methods.
pub struct LegacyRenderer(vello::Renderer);

impl LegacyRenderer {
    pub(crate) fn new(
        device: &wgpu::Device,
        options: LegacyRendererOptions,
    ) -> Result<Self, vello::Error> {
        vello::Renderer::new(
            device,
            vello::RendererOptions {
                use_cpu: options.use_cpu,
                antialiasing_support: vello::AaSupport::area_only(),
                num_init_threads: options.num_init_threads,
                pipeline_cache: options.pipeline_cache,
                buffer_sizes: options.buffer_sizes,
            },
        )
        .map(Self)
    }

    /// Renders `recording` into `view`, area-AA, clearing to `base_color`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_recording(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        recording: &Recording,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
        base_color: peniko::Color,
    ) -> Result<Option<LegacyBumpReadback>, vello::Error> {
        self.0
            .render_to_texture(
                device,
                queue,
                recording.legacy_scene(),
                view,
                &vello::RenderParams {
                    base_color,
                    width,
                    height,
                    antialiasing_method: vello::AaConfig::Area,
                },
            )
            .map(|readback| readback.map(LegacyBumpReadback))
    }

    /// Resolves `readbacks`' deferred bump-buffer feedback and returns the
    /// positions of the renders that overflowed; each overflowed render's
    /// buffers already grew to the reported demand, so re-issuing the same
    /// recording cannot overflow again.
    pub(crate) fn verify_bump_readbacks(
        &mut self,
        device: &wgpu::Device,
        readbacks: Vec<LegacyBumpReadback>,
    ) -> Result<Vec<usize>, vello::Error> {
        self.0.verify_bump_readbacks(
            device,
            readbacks.into_iter().map(|ticket| ticket.0).collect(),
        )
    }

    /// Seeds the bump buffers from a `width`×`height` render target's tile
    /// grid. Denser scenes still grow from GPU feedback; the seed only
    /// merges over demand, it never truncates it.
    pub(crate) fn seed_bump_buffer_sizes(&mut self, width: u32, height: u32) {
        self.0
            .set_buffer_sizes(Some(vello::BumpBufferSizes::for_target(width, height)));
    }

    /// Registers `texture` in the image table and returns its handle.
    pub(crate) fn register_texture(&mut self, texture: wgpu::Texture) -> peniko::ImageData {
        self.0.register_texture(texture)
    }

    /// Points `image` at `texture`'s data (or clears the override on `None`).
    pub(crate) fn override_image(
        &mut self,
        image: &peniko::ImageData,
        texture: Option<wgpu::TexelCopyTextureInfoBase<wgpu::Texture>>,
    ) {
        let _ = self.0.override_image(image, texture);
    }

    /// Unregisters a texture previously registered through
    /// [`Self::register_texture`].
    pub(crate) fn unregister_texture(&mut self, image: peniko::ImageData) {
        self.0.unregister_texture(image);
    }
}

/// A submitted render's deferred bump-buffer feedback ticket, resolved a
/// frame later through [`LegacyRenderer::verify_bump_readbacks`]. An
/// overflow is never a failed render: the re-issued render's result is
/// what reaches the screen.
pub(crate) struct LegacyBumpReadback(vello::BumpReadback);

impl LegacyBumpReadback {
    /// Whether the feedback download resolved. A `false` here advances with
    /// a device poll — never a blocking wait — before the frame drains.
    pub(crate) fn is_ready(&self) -> bool {
        self.0.is_ready()
    }

    /// The `queue.submit` index carrying this readback's buffer copy, for
    /// the runner's `PollType::Wait` completion watch.
    #[cfg(feature = "winit")]
    pub(crate) fn submission_index(&self) -> wgpu::SubmissionIndex {
        self.0.submission_index()
    }
}

/// Rasterizes `path` blurred at `sigma` into `width`×`height` premultiplied
/// pixels — the CPU half of the blurred-silhouette shadow path, for
/// silhouettes the legacy engine cannot express as a uniform rounded rect.
///
/// `offset` translates `path` into raster space (the inflated raster bounds'
/// negated origin). Cache key, padding and raster behavior live in
/// `renderer::metadata` — this is only the rasterize call itself.
pub(crate) fn rasterize_blurred_silhouette(
    path: &kurbo::BezPath,
    offset: (f64, f64),
    sigma: f64,
    color: peniko::Color,
    width: u16,
    height: u16,
) -> peniko::ImageData {
    let mut raster = vello_cpu::RenderContext::new(width, height);
    raster.set_transform(kurbo::Affine::translate(offset));
    raster.push_filter_layer(vello_common::filter_effects::Filter::from_primitive(
        vello_common::filter_effects::FilterPrimitive::GaussianBlur {
            std_deviation: sigma as f32,
            edge_mode: vello_common::filter_effects::EdgeMode::None,
        },
    ));
    raster.set_paint(color);
    raster.fill_path(path);
    raster.pop_layer();
    let mut pixmap = vello_cpu::Pixmap::new(width, height);
    raster.render(&mut pixmap, &mut vello_cpu::Resources::default());
    peniko::ImageData {
        data: peniko::Blob::from(pixmap.data_as_u8_slice().to_vec()),
        format: peniko::ImageFormat::Rgba8,
        alpha_type: peniko::ImageAlphaType::AlphaPremultiplied,
        width: u32::from(width),
        height: u32::from(height),
    }
}
