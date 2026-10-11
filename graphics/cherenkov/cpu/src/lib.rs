//! `cherenkov-cpu`: the CPU raster backend for the Cherenkov 2D rendering
//! engine.
//!
//! The shared front end lives in the [`cherenkov`] crate: [`Engine`],
//! [`Surface`], [`Layer`], the layer tree and the render thread's loop are
//! all generic over [`Backend`]. This crate supplies the render side only —
//! [`Raster`]'s [`Backend`] implementation drives a rayon worker pool on
//! the render thread.
//!
//! Framebuffers are premultiplied linear Display P3, one f32 per channel,
//! rasterized in horizontal bands of 16 rows by an exact signed-area
//! coverage accumulator (font-rs / vello-cpu style): every flattened edge
//! deposits trapezoid areas into a row accumulator and a prefix sum turns
//! it into winding-weighted coverage. Unlike the oracle's per-pixel
//! geometric area, the accumulator is exact only for polygons that do not
//! self-overlap inside a single pixel.
//!
//! Pixel work is band-bounded: a [`RasterTarget::Bands`] surface keeps no
//! framebuffer at all and streams each finished 16-row band to the host's
//! sink, so peak pixel memory is one band plus its apron. An
//! [`Offscreen`] surface keeps exactly one full-frame buffer, in the
//! [`OffscreenFormat`] the host asked for — `LinearF16` stores f16
//! directly, rounding once where readback used to round.
//!
//! Measured on the render corpus, f16 output storage costs +0.0013 mean
//! FLIP versus keeping f32 (0.00406 vs 0.00278).
//!
//! ```no_run
//! use cherenkov::{Draw, Engine, Offscreen, OffscreenFormat, WorkingColor};
//! use cherenkov::kurbo::Rect;
//! use cherenkov_cpu::{Raster, RasterConfig};
//!
//! let engine = Engine::<Raster>::new(RasterConfig::default())?;
//! let surface = engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16), || {})?;
//! surface.update(|tx| {
//!     tx[surface.root()].content(
//!         surface.record(|c| c.fill(Rect::new(0., 0., 64., 64.), WorkingColor::WHITE)),
//!     );
//! });
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod names;
mod render;

use cherenkov::{Backend, EngineError, Offscreen, OffscreenFormat};

/// CPU worker information for provenance.
#[derive(Clone, Debug)]
pub struct RasterInfo {
    /// Number of rayon worker threads.
    pub threads: usize,
    /// The composite kernel in use (`"scalar"` in this slice).
    pub simd: &'static str,
    /// Host CPU model name, best effort.
    pub cpu: Option<String>,
}

/// Configuration for the CPU raster engine.
#[derive(Clone, Debug, Default)]
pub struct RasterConfig {
    /// Worker thread count for the banded rasterizer. `None` uses the
    /// rayon default (one thread per logical core).
    pub threads: Option<usize>,
    /// Memory budgets; only `budget.cpu` is used for resident images and cached glyph masks.
    /// Surface framebuffers and in-flight frame data are not evictable caches.
    pub budget: cherenkov::Budget,
}

/// The surface targets [`Raster`] draws into: an [`Offscreen`]
/// framebuffer, or a [`Bands`] streaming sink.
#[derive(Debug)]
pub enum RasterTarget {
    /// An offscreen framebuffer.
    Offscreen(Offscreen),
    /// A band-streaming sink.
    Bands(Bands),
}

impl From<Offscreen> for RasterTarget {
    fn from(offscreen: Offscreen) -> Self {
        Self::Offscreen(offscreen)
    }
}

impl From<Bands> for RasterTarget {
    fn from(bands: Bands) -> Self {
        Self::Bands(bands)
    }
}

/// A band-streaming surface target for hosts that consume pixels in row
/// order — a display driver's row DMA, an image encoder, a tile printer.
///
/// No full-frame buffer exists on the surface: each rasterized band is
/// handed to the sink and its storage reused, so peak pixel memory is one
/// band plus its apron. Such surfaces are not readable: `readback`
/// returns an error.
pub struct Bands {
    /// Surface size in pixels.
    pub size: (u32, u32),
    /// The pixel format bands are delivered in.
    pub format: OffscreenFormat,
    /// The host's band consumer, called in row order on the render
    /// thread. Pixels are borrowed for the call's duration; copy what is
    /// kept.
    pub sink: Box<dyn FnMut(Band<'_>) + Send>,
}

impl Bands {
    /// A streaming target of `size` pixels delivering bands in `format`
    /// to `sink`.
    pub fn new(
        size: (u32, u32),
        format: OffscreenFormat,
        sink: impl FnMut(Band<'_>) + Send + 'static,
    ) -> Self {
        Self {
            size,
            format,
            sink: Box::new(sink),
        }
    }
}

impl std::fmt::Debug for Bands {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bands")
            .field("size", &self.size)
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

/// One rasterized band handed to a [`Bands`] sink.
#[derive(Debug)]
pub struct Band<'a> {
    /// First device row of this band.
    pub y: u32,
    /// The band's pixels, `width * rows` in row order, premultiplied
    /// linear Display P3 in the target's format.
    pub pixels: BandPixels<'a>,
}

/// Band pixels in the surface's output format.
#[derive(Clone, Copy, Debug)]
pub enum BandPixels<'a> {
    /// One `f32` per channel (`LinearF32`).
    F32(&'a [[f32; 4]]),
    /// One `f16` per channel (`LinearF16`).
    F16(&'a [[half::f16; 4]]),
}

pub use render::present::{present_linear_p3, present_srgb8};

/// The CPU raster backend: renders the shared front end's layer trees
/// band by band on a rayon pool.
#[derive(Clone, Copy, Debug, Default)]
pub struct Raster;

impl cherenkov::ProjectiveLayers for Raster {}
impl cherenkov::BackdropSampling for Raster {}
impl cherenkov::Uploads<cherenkov::Rgba8> for Raster {}
impl cherenkov::Uploads<cherenkov::Rgba16F> for Raster {}

impl cherenkov::Filters for Raster {
    fn remove_filter(renderer: &mut Self::Renderer, id: cherenkov::FilterId) {
        renderer.filters.remove(id);
    }
}

impl<F> cherenkov::Runs<F> for Raster
where
    F: filtrate_core::CpuFilter + cherenkov::RenderTransfer + Send + Sync,
{
    fn add_filter(renderer: &mut Self::Renderer, id: cherenkov::FilterId, filter: F) {
        renderer.filters.add(id, filter);
    }
}

impl cherenkov::Backdrop for Raster {
    fn add_backdrop_group(
        renderer: &mut Self::Renderer,
        surface: cherenkov::SurfaceId,
        id: cherenkov::BackdropId,
        spec: cherenkov::BackdropSpec,
    ) {
        renderer.filters.add_backdrop_group(surface, id, spec);
    }

    fn remove_backdrop_group(
        renderer: &mut Self::Renderer,
        surface: cherenkov::SurfaceId,
        id: cherenkov::BackdropId,
    ) {
        renderer.filters.remove_backdrop_group(surface, id);
    }
}

impl<K, F> cherenkov::BackdropRuns<K, F> for Raster
where
    K: filtrate_core::kind::Kind,
    F: cherenkov::BackdropChain<K>
        + filtrate_core::CpuFilter
        + cherenkov::RenderTransfer
        + Send
        + Sync,
{
    fn add_filtered_backdrop_group(
        renderer: &mut Self::Renderer,
        surface: cherenkov::SurfaceId,
        id: cherenkov::BackdropId,
        filter: F,
        spec: cherenkov::BackdropSpec,
    ) {
        renderer
            .filters
            .add_filtered_backdrop_group(surface, id, filter, spec);
    }
}

impl cherenkov::Target for Raster {
    type Queue = cherenkov::EngineQueue<Self>;
    type Install = cherenkov::InstallOp<Self>;
}

impl Backend for Raster {
    type Config = RasterConfig;
    type Info = RasterInfo;
    type Target = RasterTarget;
    type Renderer = render::RasterRenderer;

    #[cfg(not(target_arch = "wasm32"))]
    fn init(config: RasterConfig) -> Result<(Self::Renderer, Self::Info), EngineError> {
        render::init(&config)
    }

    #[cfg(target_arch = "wasm32")]
    fn init(
        config: RasterConfig,
    ) -> impl core::future::Future<Output = Result<(Self::Renderer, Self::Info), EngineError>> {
        let mut config = Some(config);
        core::future::poll_fn(move |_| {
            core::task::Poll::Ready(render::init(
                &config.take().expect("init future is only polled once"),
            ))
        })
    }
}
