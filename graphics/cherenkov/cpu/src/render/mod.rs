//! The render side of the [`Raster`](crate::Raster) backend: sole owner
//! of the framebuffers and the worker pool, driven by the shared front
//! end's render loop.

mod account;
mod bins;
mod bitmap;
mod blend;
mod colr;
pub mod coverage;
mod damage;
mod filter;
mod font;
mod geometry;
mod glyph;
mod gradient;
mod image;
mod mesh;
#[cfg(any(test, feature = "testing"))]
pub use image::CpuImage;
#[cfg(not(any(test, feature = "testing")))]
use image::CpuImage;
use std::sync::Arc;
mod intersect;
mod lower;
mod paint;
mod prepared;
pub mod present;
mod projective;
mod raster;
mod shadow;
mod span;
mod tiles;
mod topology;

// The `testing` feature's surface: the span kernels and the types their
// signatures name. `render` is a private module, so nothing is reachable
// without `crate::testing`'s re-export, which does not exist without the
// feature.
#[cfg(any(test, feature = "testing"))]
pub use lower::GeometryChange;
#[cfg(any(test, feature = "testing"))]
pub use lower::{ClipMask, ClipRef};
#[cfg(any(test, feature = "testing"))]
pub use paint::{PaintData, paint_data};
#[cfg(any(test, feature = "testing"))]
pub use raster::{ClipCoverage, ClipRun, Edge, composite_span};
#[cfg(any(test, feature = "testing"))]
pub use raster::{composite_solid, convert_span};

use rustc_hash::{FxHashMap, FxHashSet};

use cherenkov::{
    BackdropId, ContentOp, EngineError, FontData, FontId, Frame, FrameRedraw, FrameStats, ImageId,
    ImageUpload, LayerId, MemoryUsage, Pressure, Readback, RenderError, Renderer, ResourceError,
    ResourceId, SurfaceError, SurfaceId, SurfaceInfo, Visibility,
};
use lower::{ContentData, Item, Lowering};

use crate::{Band, RasterConfig, RasterInfo, RasterTarget};

/// The largest surface dimension the CPU framebuffer supports.
const MAX_SURFACE: u32 = 16384;

/// A surface's pixel destination.
enum Output {
    /// Full-frame storage in `LinearF32`: the readback buffer.
    F32(Vec<[f32; 4]>),
    /// Full-frame storage in `LinearF16`: the readback buffer in the
    /// host's requested format.
    F16(Vec<[half::f16; 4]>),
    /// No frame storage: finished bands stream to the host's sink in row
    /// order. `emit` is the `LinearF16` conversion buffer.
    Stream {
        format: cherenkov::OffscreenFormat,
        sink: Box<dyn FnMut(Band<'_>) + Send>,
        emit: Vec<[half::f16; 4]>,
    },
}

/// One surface's render-thread state.
struct SurfaceState {
    damage: damage::Damage,
    size: (u32, u32),
    /// Where finished pixels land; only `Offscreen` targets retain a
    /// full-frame buffer, in the format the host asked for.
    output: Output,
    refresh: cherenkov::RefreshRange,
    /// Per-layer content caches; the sampled layer state lives in the
    /// front end's [`cherenkov::SurfaceTree`].
    layers: FxHashMap<LayerId, ContentData>,
    filters: Vec<u64>,
    /// Backdrop groups referenced by the last frame, by group id.
    groups: Vec<u64>,
    anchor_scratch: lower::AnchorScratch,
    /// The largest live pixel-buffer bytes in any band that ran a
    /// backdrop capture last frame (window, isolation stack and capture
    /// buffers). 0 when no capture ran.
    backdrop_capture_peak: u64,
    /// Peak clip compiler, sample and pool storage across last frame's workers.
    clip_scratch_peak: u64,
    /// Projective layers' realized local images, by layer, one per
    /// density bucket.
    projective: FxHashMap<LayerId, Vec<projective::Entry>>,
    /// The host's announced visibility as the render loop applied it. A
    /// hidden surface is in no frame, and its filters ask for no redraw.
    visibility: Visibility,
    /// The surface's host wake-up: the filters its frames run wake the
    /// host through it, so they stop waking the moment the host hides the
    /// surface.
    waker: cherenkov::CompletionWaker,
}

impl SurfaceState {
    fn rendered_bands(&self) -> u32 {
        let count = match &self.output {
            Output::Stream { .. } => (self.size.1 as usize).div_ceil(raster::BAND_H),
            Output::F32(_) | Output::F16(_) => {
                usize::try_from(self.damage.bands().pixel_area()).expect("band count fits usize")
            }
        };
        u32::try_from(count).expect("surface band count fits u32")
    }

    /// Heap bytes of the output storage (`Stream` keeps none).
    const fn output_bytes(&self) -> u64 {
        match &self.output {
            Output::F32(fb) => (fb.capacity() * size_of::<[f32; 4]>()) as u64,
            Output::F16(fb) => (fb.capacity() * size_of::<[half::f16; 4]>()) as u64,
            Output::Stream { .. } => 0,
        }
    }

    /// Heap bytes of the surface's own band-format working buffer.
    const fn band_bytes(&self) -> u64 {
        self.clip_scratch_peak
            + match &self.output {
                Output::Stream { emit, .. } => {
                    (emit.capacity() * size_of::<[half::f16; 4]>()) as u64
                }
                _ => 0,
            }
    }
}

/// All render-thread state: the [`Raster`](crate::Raster) backend's
/// [`Renderer`] implementation.
pub struct RasterRenderer {
    pool: rayon::ThreadPool,
    /// The host CPU's SIMD level, detected once at construction: the
    /// level every hot pass is dispatched under.
    level: fearless_simd::Level,
    surfaces: FxHashMap<SurfaceId, SurfaceState>,
    pub(super) filters: filter::Registry,
    fonts: FxHashMap<u64, font::Font>,
    bitmap_fonts: FxHashMap<u64, Arc<bitmap::BitmapFont>>,
    images: FxHashMap<u64, Arc<CpuImage>>,
    image_budget: u64,
    /// The glyph mask cache, bounded by `Budget::cpu`.
    glyph_cache: glyph::GlyphCache,
    /// Decoded colour-font bitmaps, bounded by `Budget::cpu`.
    bitmap_cache: bitmap::BitmapCache,
    /// Fill and stroke geometry offered by realizations dropped or
    /// rebuilt this frame; empty between frames.
    geometry: geometry::GeometryCache,
    /// Reusable build storage for retained geometry; high-water
    /// capacity persists across frames.
    edge_scratch: tiles::EdgeScratch,
    /// Bumped on every image replacement: the one change to what a
    /// current projective image read that arrives without a tree edit.
    /// A removal only runs once no installed content draws the image
    /// (#199), so it does not count.
    image_replacements: u64,
    /// Counts rendered frames; the projective cache's recency clock.
    frame_count: u64,
    /// Rebuilt for each raster call; capacity is shared across surfaces.
    /// Boxed so lending it to a surface render moves a pointer, not the
    /// index itself — a by-value move of the struct is a `memcpy` call
    /// every frame. `None` only while a surface render holds it.
    band_index: Option<Box<bins::FrameIndex>>,
}

impl std::fmt::Debug for RasterRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RasterRenderer")
            .field("surfaces", &self.surfaces.len())
            .field("fonts", &self.fonts.len())
            .field("images", &self.images.len())
            .field("image_budget", &self.image_budget)
            .finish_non_exhaustive()
    }
}

/// Runs on the render thread once: builds the worker pool, returning the
/// backend's [`Renderer`].
///
/// # Errors
/// [`EngineError::Backend`] when the pool cannot be built.
pub fn init(config: &RasterConfig) -> Result<(RasterRenderer, RasterInfo), EngineError> {
    let builder = rayon::ThreadPoolBuilder::new()
        .num_threads(cherenkov::workers::count(config.threads)?.get())
        .thread_name(|i| format!("cherenkov-raster-{i}"));
    builder
        .build()
        .map(|pool| {
            let level = cherenkov_simd::detect();
            let info = RasterInfo {
                threads: pool.current_num_threads(),
                simd: level_name(level),
                cpu: cpu_model(),
            };
            (
                RasterRenderer {
                    pool,
                    level,
                    surfaces: FxHashMap::default(),
                    filters: filter::Registry::default(),
                    fonts: FxHashMap::default(),
                    bitmap_fonts: FxHashMap::default(),
                    images: FxHashMap::default(),
                    image_budget: config.budget.cpu.0,
                    glyph_cache: glyph::GlyphCache::new(config.budget.cpu.0),
                    bitmap_cache: bitmap::BitmapCache::new(config.budget.cpu.0),
                    geometry: geometry::GeometryCache::default(),
                    edge_scratch: tiles::EdgeScratch::default(),
                    image_replacements: 0,
                    frame_count: 0,
                    band_index: Some(Box::default()),
                },
                info,
            )
        })
        .map_err(|e| EngineError::Backend(format!("rayon pool: {e}")))
}

/// The `RasterInfo::simd` string for `level` — the CPU level the span
/// kernels actually dispatch to. The variant accessors are cfg-gated to
/// the same conditions as the variants they unwrap (so a forced-fallback
/// build reports `"fallback"` through `is_fallback`); an unrecognised
/// level is a bug and panics.
fn level_name(level: fearless_simd::Level) -> &'static str {
    #[cfg(target_arch = "aarch64")]
    if level.as_neon().is_some() {
        return "neon";
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    if level.as_wasm_simd128().is_some() {
        return "simd128";
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if level.as_avx2().is_some() {
        return "avx2";
    }
    #[cfg(all(
        any(target_arch = "x86", target_arch = "x86_64"),
        not(all(
            target_feature = "avx2",
            target_feature = "bmi1",
            target_feature = "bmi2",
            target_feature = "cmpxchg16b",
            target_feature = "f16c",
            target_feature = "fma",
            target_feature = "lzcnt",
            target_feature = "movbe",
            target_feature = "popcnt",
            target_feature = "xsave"
        ))
    ))]
    if level.as_sse4_2().is_some() {
        return "sse4.2";
    }
    if level.is_fallback() {
        return "fallback";
    }
    unreachable!("unsupported SIMD level {level:?}")
}

/// The host CPU model, best effort.
fn cpu_model() -> Option<String> {
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    for line in cpuinfo.lines() {
        if let Some(v) = line
            .strip_prefix("model name")
            .and_then(|s| s.split(':').nth(1))
        {
            return Some(v.trim().to_owned());
        }
    }
    None
}

impl RasterRenderer {
    /// [`Renderer::add_image`]'s work, which never waits on a device.
    fn register_image(&mut self, id: ImageId, image: &ImageUpload) -> Result<(), ResourceError> {
        let decoded = Arc::new(CpuImage::decode(image)?);
        self.images.insert(id.raw(), decoded);
        self.refresh_cache_budgets();
        Ok(())
    }

    /// [`Renderer::replace_image`]'s work, which never waits on a device.
    fn swap_image(&mut self, id: ImageId, image: &ImageUpload) -> Result<(), ResourceError> {
        let current = self
            .images
            .get(&id.raw())
            .expect("replace targets a registered image");
        let resized = if (current.width, current.height) == (image.width, image.height) {
            CpuImage::validate(image)?;
            None
        } else {
            Some(Arc::new(CpuImage::decode(image)?))
        };
        // Retained paint operands share the pixels and lowering resolved
        // the dimensions: content sampling the image is lowered again.
        self.image_replacements += 1;
        for surface in self.surfaces.values_mut() {
            for content in surface.layers.values_mut() {
                let _ = content.invalidate_image(id);
            }
        }
        let slot = self
            .images
            .get_mut(&id.raw())
            .expect("replace targets a registered image");
        if let Some(resized) = resized {
            *slot = resized;
            self.refresh_cache_budgets();
        } else {
            // Discarding the content above released every retained operand
            // sharing the pixels. An operand that survived is an engine
            // defect; it is reported as this replacement's rejection, which
            // keeps the previous pixels and fails the renders that draw the
            // image, instead of panicking the render thread.
            Arc::get_mut(slot)
                .ok_or_else(|| {
                    ResourceError::Image(format!(
                        "image {} is still shared by retained paint operands after its content was discarded",
                        id.raw()
                    ))
                })?
                .overwrite(image);
        }
        Ok(())
    }
}

impl Renderer for RasterRenderer {
    type Target = RasterTarget;
    type Font = font::PreparedFont;
    type FrameCommit = ();

    fn create_surface(
        &mut self,
        id: SurfaceId,
        target: RasterTarget,
        // The raster backend has no completion that lands after a render;
        // its filters' wakes are gated on the surface's visibility.
        waker: cherenkov::CompletionWaker,
    ) -> Result<SurfaceInfo, SurfaceError> {
        let (size, output, readable, refresh) = match target {
            RasterTarget::Offscreen(offscreen) => {
                let (size, refresh) = (offscreen.size, offscreen.refresh);
                let pixels = (size.0 * size.1) as usize;
                let output = match offscreen.format {
                    cherenkov::OffscreenFormat::LinearF32 => Output::F32(vec![[0.0; 4]; pixels]),
                    cherenkov::OffscreenFormat::LinearF16 => {
                        Output::F16(vec![[half::f16::ZERO; 4]; pixels])
                    }
                };
                (size, output, true, refresh)
            }
            RasterTarget::Bands(bands) => {
                let (size, format) = (bands.size, bands.format);
                let output = Output::Stream {
                    format: bands.format,
                    sink: bands.sink,
                    // Exactly one band's worth for f16 emission, so
                    // `memory()` is stable. f32 streams borrow the scratch.
                    emit: match format {
                        cherenkov::OffscreenFormat::LinearF16 => Vec::with_capacity(
                            size.0 as usize * raster::BAND_H.min(size.1 as usize),
                        ),
                        cherenkov::OffscreenFormat::LinearF32 => Vec::new(),
                    },
                };
                (size, output, false, cherenkov::DEFAULT_REFRESH)
            }
        };
        if size.0 > MAX_SURFACE || size.1 > MAX_SURFACE {
            return Err(SurfaceError::TooLarge {
                width: size.0,
                height: size.1,
                max: MAX_SURFACE,
            });
        }
        self.surfaces.insert(
            id,
            SurfaceState {
                damage: damage::Damage::default(),
                size,
                output,
                refresh,
                layers: FxHashMap::default(),
                filters: Vec::new(),
                groups: Vec::new(),
                anchor_scratch: lower::AnchorScratch::default(),
                backdrop_capture_peak: 0,
                clip_scratch_peak: 0,
                projective: FxHashMap::default(),
                visibility: Visibility::Visible,
                waker,
            },
        );
        Ok(SurfaceInfo {
            max_dimension: MAX_SURFACE,
            size,
            readable,
            // The raster backend has no window targets.
            presents: false,
        })
    }

    fn resize_surface(&mut self, id: SurfaceId, size: (u32, u32)) {
        let Some(state) = self.surfaces.get_mut(&id) else {
            return;
        };
        state.size = size;
        state.damage.forget();
        let pixels = (size.0 * size.1) as usize;
        match &mut state.output {
            Output::F32(fb) => *fb = vec![[0.0; 4]; pixels],
            Output::F16(fb) => *fb = vec![[half::f16::ZERO; 4]; pixels],
            Output::Stream { format, emit, .. } => {
                *emit = match format {
                    cherenkov::OffscreenFormat::LinearF16 => {
                        Vec::with_capacity(size.0 as usize * raster::BAND_H.min(size.1 as usize))
                    }
                    cherenkov::OffscreenFormat::LinearF32 => Vec::new(),
                };
            }
        }
    }

    fn destroy_surface(&mut self, id: SurfaceId) {
        self.surfaces.remove(&id);
    }

    fn set_visibility(&mut self, id: SurfaceId, visibility: Visibility) {
        // The filters' wakes already follow the announced visibility
        // through the surface's waker; this decides what counts in
        // `FrameRedraw` and what each frame housekeeps.
        self.surfaces
            .get_mut(&id)
            .expect("visibility of a created surface")
            .visibility = visibility;
    }

    /// Validates font data and detects colour-glyph sources.
    fn prepare_font(font: FontData) -> Result<font::PreparedFont, ResourceError> {
        use skrifa::raw::TableProvider as _;
        let parsed = skrifa::FontRef::from_index(&font.data, font.index)
            .map_err(|e| ResourceError::Font(format!("{e}")))?;
        let has_colr = parsed.colr().is_ok();
        let bitmap = bitmap::BitmapFont::detect(&font.data, font.index)?.map(Arc::new);
        Ok(font::PreparedFont {
            data: font,
            has_colr,
            bitmap,
        })
    }

    fn add_font(&mut self, id: FontId, font: font::PreparedFont) {
        let has_bitmap = font.bitmap.is_some();
        if let Some(bitmap) = font.bitmap {
            self.bitmap_fonts.insert(id.raw(), bitmap);
        } else {
            self.bitmap_fonts.remove(&id.raw());
        }
        self.fonts.insert(
            id.raw(),
            font::Font {
                data: font.data,
                has_colr: font.has_colr,
                has_bitmap,
                colr: FxHashMap::default(),
            },
        );
    }

    fn remove_font(&mut self, id: FontId) {
        self.fonts.remove(&id.raw());
        self.bitmap_fonts.remove(&id.raw());
        self.bitmap_cache.remove_font(id.raw());
        self.refresh_cache_budgets();
        for surface in self.surfaces.values_mut() {
            for content in surface.layers.values_mut() {
                content.invalidate();
            }
        }
    }

    fn image_limits(&self) -> cherenkov::ImageLimits {
        cherenkov::ImageLimits {
            max_dimension: u32::MAX,
            // The image budget: every registered image together holds at
            // most the CPU budget, decoded as f32 RGBA at sixteen bytes a
            // pixel. The engine admits registrations against it, and the
            // glyph caches take what the images leave.
            max_bytes: self.image_budget,
            storage: cherenkov::ImageStorage::CPU,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn add_image(&mut self, id: ImageId, image: ImageUpload) -> Result<(), ResourceError> {
        self.register_image(id, &image)
    }

    #[cfg(target_arch = "wasm32")]
    fn add_image(
        &mut self,
        id: ImageId,
        image: ImageUpload,
    ) -> impl core::future::Future<Output = Result<(), ResourceError>> {
        core::future::ready(self.register_image(id, &image))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn replace_image(&mut self, id: ImageId, image: ImageUpload) -> Result<(), ResourceError> {
        self.swap_image(id, &image)
    }

    #[cfg(target_arch = "wasm32")]
    fn replace_image(
        &mut self,
        id: ImageId,
        image: ImageUpload,
    ) -> impl core::future::Future<Output = Result<(), ResourceError>> {
        core::future::ready(self.swap_image(id, &image))
    }

    fn samples(&self, surface: SurfaceId, resource: ResourceId) -> bool {
        self.surfaces.get(&surface).is_some_and(|state| {
            state
                .layers
                .values()
                .any(|content| content.references(resource))
        })
    }

    fn remove_image(&mut self, id: ImageId) {
        self.images.remove(&id.raw());
        self.refresh_cache_budgets();
        for surface in self.surfaces.values_mut() {
            for content in surface.layers.values_mut() {
                content.invalidate();
            }
        }
    }

    fn set_content(
        &mut self,
        surface: SurfaceId,
        layer: LayerId,
        content: Option<ContentOp>,
    ) -> Option<cherenkov::Picture> {
        let state = self.surfaces.get_mut(&surface)?;
        match content {
            Some(ContentOp::Replace(list)) => state
                .layers
                .insert(layer, ContentData::new(list))
                .map(|previous| {
                    lower::harvest(&mut self.geometry, &previous);
                    previous.into_picture()
                }),
            Some(ContentOp::Update(updates)) => {
                state
                    .layers
                    .get_mut(&layer)
                    .expect("slot update targets a layer without content")
                    .update(updates);
                None
            }
            Some(ContentOp::Picture(picture)) => state
                .layers
                .insert(layer, ContentData::picture(picture))
                .map(|previous| {
                    lower::harvest(&mut self.geometry, &previous);
                    previous.into_picture()
                }),
            None => state
                .layers
                .remove(&layer)
                .map(cherenkov::lowering::Content::into_picture),
        }
    }

    fn remove_layer(&mut self, surface: SurfaceId, layer: LayerId) {
        if let Some(state) = self.surfaces.get_mut(&surface) {
            state.layers.remove(&layer);
            state.projective.remove(&layer);
        }
    }

    /// Lowers and rasterizes every changed surface.
    #[cfg(not(target_arch = "wasm32"))]
    fn render(
        &mut self,
        frame: &Frame<'_>,
        stats: &mut FrameStats,
    ) -> Result<(FrameRedraw, Self::FrameCommit), RenderError> {
        self.render_frame(frame, stats).map(|redraw| (redraw, ()))
    }

    #[cfg(target_arch = "wasm32")]
    fn render(
        &mut self,
        frame: &Frame<'_>,
        stats: &mut FrameStats,
    ) -> impl core::future::Future<Output = Result<(FrameRedraw, Self::FrameCommit), RenderError>>
    {
        core::future::ready(self.render_frame(frame, stats).map(|redraw| (redraw, ())))
    }

    /// Materializes a surface's output buffer into `Readback` pixels.
    /// `LinearF16` stores already round once; the readback only widens.
    /// Band-streaming surfaces have no frame buffer to read.
    #[cfg(not(target_arch = "wasm32"))]
    fn readback(&mut self, surface: SurfaceId) -> Result<Readback, RenderError> {
        let Some(state) = self.surfaces.get(&surface) else {
            return Err(RenderError::Readback("unknown surface".into()));
        };
        let pixels = match &state.output {
            Output::F32(fb) => fb.clone(),
            Output::F16(fb) => fb.iter().map(|px| px.map(half::f16::to_f32)).collect(),
            Output::Stream { .. } => {
                return Err(RenderError::Readback(
                    "band-streaming surfaces are not readable".into(),
                ));
            }
        };
        Ok(Readback {
            width: state.size.0,
            height: state.size.1,
            pixels,
        })
    }

    #[cfg(target_arch = "wasm32")]
    fn readback(
        &mut self,
        surface: SurfaceId,
    ) -> impl core::future::Future<Output = Result<Readback, RenderError>> {
        let Some(state) = self.surfaces.get(&surface) else {
            return core::future::ready(Err(RenderError::Readback("unknown surface".into())));
        };
        let pixels = match &state.output {
            Output::F32(fb) => fb.clone(),
            Output::F16(fb) => fb.iter().map(|px| px.map(half::f16::to_f32)).collect(),
            Output::Stream { .. } => {
                return core::future::ready(Err(RenderError::Readback(
                    "band-streaming surfaces are not readable".into(),
                )));
            }
        };
        core::future::ready(Ok(Readback {
            width: state.size.0,
            height: state.size.1,
            pixels,
        }))
    }

    /// Memory usage across output targets, band working buffers, retained
    /// layer content, registered images and the glyph caches.
    fn memory(&self) -> MemoryUsage {
        let mut categories = account::Categories::default();
        for surface in self.surfaces.values() {
            categories.output += surface.output_bytes();
            categories.retained += surface.anchor_scratch.heap_bytes();
            categories.retained += surface.damage.heap_bytes();
            categories.retained += surface
                .layers
                .values()
                .map(|content| account::content_bytes(content) + lower::silhouette_bytes(content))
                .sum::<u64>();
        }
        categories.bands = self
            .surfaces
            .values()
            .map(SurfaceState::band_bytes)
            .sum::<u64>()
            + self.band_index.as_ref().map_or(0, |index| {
                size_of::<bins::FrameIndex>() as u64 + index.heap_bytes()
            })
            + self.edge_scratch.heap_bytes();
        categories.glyphs = self.glyph_cache.bytes();
        categories.images = self.images.values().map(|image| image.bytes()).sum();
        categories.colr = self.fonts.values().map(font::Font::colr_bytes).sum();
        categories.bitmaps = self.bitmap_cache.bytes();
        categories.geometry = self.geometry.bytes();
        categories.projective = self.projective_bytes();
        tracing::debug!(
            target: "cherenkov_cpu::memory",
            output = categories.output,
            bands = categories.bands,
            retained = categories.retained,
            images = categories.images,
            glyphs = categories.glyphs,
            colr = categories.colr,
            bitmaps = categories.bitmaps,
            geometry = categories.geometry,
            projective = categories.projective,
            "memory usage",
        );
        // Backdrop captures are transient: the reported value is the
        // peak live pixel-buffer bytes of the last frame's capture bands,
        // in premultiplied `f32` (`linear-f32`).
        let backdrop_captures: u64 = self
            .surfaces
            .values()
            .map(|surface| surface.backdrop_capture_peak)
            .sum();
        MemoryUsage {
            gpu: cherenkov::Bytes(0),
            cpu: cherenkov::Bytes(categories.total()),
            glyph_cache: cherenkov::Bytes(categories.glyphs + categories.colr + categories.bitmaps),
            // The CPU backend has no tile memory to compose in.
            tile_reserved: cherenkov::Bytes(0),
            tile_committed: None,
            backdrop_captures: cherenkov::Bytes(backdrop_captures),
            backdrop_capture_format: (backdrop_captures > 0).then_some("linear-f32"),
            // CPU filters run on the band's transient windows, accounted
            // under `cpu`; no chain retains a target of its own.
            filter_targets: cherenkov::Bytes(0),
            // CPU-side auxiliary image data is already in the chain and
            // accounted under `cpu`; no filter uploads an image here.
            filter_images: cherenkov::Bytes(0),
        }
    }

    fn trim(&mut self, pressure: Pressure) {
        if pressure == Pressure::Critical {
            for font in self.fonts.values_mut() {
                font.colr.clear();
            }
            self.fonts.shrink_to_fit();
            self.glyph_cache.clear();
            self.bitmap_cache.clear();
            self.geometry.clear();
            self.band_index = Some(Box::default());
            for surface in self.surfaces.values_mut() {
                for content in surface.layers.values_mut() {
                    content.trim();
                }
                surface.projective.clear();
            }
        }
    }

    /// A CPU renderer imports no native frames — retirements queue no
    /// native releases, so there is nothing to submit.
    fn submit_native_releases(&mut self) {}
}

impl RasterRenderer {
    fn render_frame(
        &mut self,
        frame: &Frame<'_>,
        stats: &mut FrameStats,
    ) -> Result<FrameRedraw, RenderError> {
        self.filters.begin_frame(frame.id, frame.time);
        self.frame_count += 1;
        for sf in frame.surfaces {
            let filter_changed = self.surfaces.get(&sf.id).is_some_and(|surface| {
                surface
                    .filters
                    .iter()
                    .any(|id| self.filters.wants_redraw(*id))
                    || surface
                        .groups
                        .iter()
                        .any(|id| self.filters.wants_redraw_group(sf.id, BackdropId::new(*id)))
            });
            if sf.changed || filter_changed {
                stats.frame = Some(frame.id);
                let mut index = self
                    .band_index
                    .take()
                    .expect("the band index is lent only for one surface render");
                let result = self.render_surface(sf, frame.id, stats, &mut index, filter_changed);
                self.band_index = Some(index);
                result?;
            }
        }
        let (used, used_groups) = self.filter_uses();
        self.evict_projective();
        self.geometry.finish_frame();
        self.edge_scratch.clear();
        self.filters.finish_frame(&used, &used_groups);
        self.update_filter_wakes();
        // Every visible surface whose filter or backdrop group still
        // runs asks for the next frame on its own entry — the animated
        // backdrop names the surface it draws into.
        let mut redraw = FrameRedraw::default();
        for (id, surface) in &self.surfaces {
            if surface.visibility != Visibility::Visible {
                continue;
            }
            if surface
                .filters
                .iter()
                .any(|fid| self.filters.wants_redraw(*fid))
                || surface
                    .groups
                    .iter()
                    .any(|gid| self.filters.wants_redraw_group(*id, BackdropId::new(*gid)))
            {
                redraw.request(*id, surface.refresh.clone());
            }
        }
        Ok(redraw)
    }

    /// Points every filter's and backdrop chain's wakes at the surfaces
    /// whose last frames ran it.
    fn update_filter_wakes(&self) {
        let mut uses: FxHashMap<u64, Vec<cherenkov::CompletionWaker>> = FxHashMap::default();
        let mut groups = FxHashMap::default();
        for (surface, state) in &self.surfaces {
            for filter in &state.filters {
                let surfaces = uses.entry(*filter).or_default();
                // Listed surface by surface: one entry per surface.
                if surfaces.last() != Some(&state.waker) {
                    surfaces.push(state.waker.clone());
                }
            }
            for group in &state.groups {
                groups.insert((surface.raw(), *group), state.waker.clone());
            }
        }
        self.filters.set_surfaces(&uses, &groups);
    }

    /// The filters and backdrop groups the visible surfaces' last frames
    /// ran: the entries housekept per frame. A hidden surface's are left
    /// alone until it is shown, when the front end redraws it whole.
    fn filter_uses(&self) -> (FxHashSet<u64>, FxHashSet<(u64, u64)>) {
        let visible = || {
            self.surfaces
                .iter()
                .filter(|(_, state)| state.visibility == Visibility::Visible)
        };
        let used = visible()
            .flat_map(|(_, state)| state.filters.iter().copied())
            .collect();
        let used_groups = visible()
            .flat_map(|(surface, state)| {
                state
                    .groups
                    .iter()
                    .map(move |group| (surface.raw(), *group))
            })
            .collect();
        (used, used_groups)
    }

    fn refresh_cache_budgets(&mut self) {
        let resident: u64 =
            self.images.values().map(|image| image.bytes()).sum::<u64>() + self.projective_bytes();
        let available = self.image_budget.saturating_sub(resident);
        self.bitmap_cache
            .set_budget(available.saturating_sub(self.glyph_cache.bytes()));
        self.glyph_cache
            .set_budget(available.saturating_sub(self.bitmap_cache.bytes()));
    }
    /// Lowers and rasterizes one surface's frame, adding the time of each
    /// phase to `stats.phases`.
    #[expect(
        clippy::many_single_char_names,
        reason = "w/h and r/g/b/a are the natural names"
    )]
    fn render_surface(
        &mut self,
        sf: &cherenkov::SurfaceFrame<'_>,
        frame: cherenkov::FrameId,
        stats: &mut FrameStats,
        index: &mut bins::FrameIndex,
        filter_changed: bool,
    ) -> Result<(), RenderError> {
        let phases = stats.phases;
        let id = sf.id;
        let Some(size) = self.surfaces.get(&id).map(|surf| surf.size) else {
            return Ok(());
        };
        // The frame's transient band-index metrics, shared by the
        // projective realizations and the surface render: each records
        // the frame-local peak.
        let mut frame_mem = bins::FrameMeter::default();
        let (projected, mut used) =
            self.realize_projective(sf, frame, stats, index, &mut frame_mem)?;
        let mut items: Vec<Item> = Vec::new();
        let glyph_reqs = self.lower_items(
            sf,
            &mut items,
            size,
            frame,
            stats,
            (None, projected),
            &mut used,
        )?;
        let Some(surf) = self.surfaces.get_mut(&id) else {
            return Ok(());
        };
        surf.filters = used.filters.into_iter().collect();
        surf.groups = used.groups.into_iter().collect();
        let (w, h) = (surf.size.0 as usize, surf.size.1 as usize);
        let [r, g, b, a] = sf.clear.components;
        let clear = [r * a, g * a, b * a, a];
        surf.damage
            .prepare(&items, h, clear, self.image_replacements, filter_changed);
        let pool = &self.pool;
        let level = self.level;
        let peak = std::sync::atomic::AtomicU64::new(0);
        let has_backdrop = !surf.groups.is_empty();
        let shade_at = cherenkov::Instant::now();
        let (draws, edges) = match &mut surf.output {
            Output::F32(fb) => pool.install(|| {
                raster::render_bands(
                    level,
                    &items,
                    index,
                    clear,
                    fb,
                    w,
                    h,
                    Some(&peak),
                    has_backdrop,
                    &mut frame_mem,
                    Some(surf.damage.bands()),
                )
            })?,
            Output::F16(out) => pool.install(|| {
                raster::render_bands_f16(
                    level,
                    &items,
                    index,
                    clear,
                    out,
                    w,
                    h,
                    Some(&peak),
                    has_backdrop,
                    &mut frame_mem,
                    Some(surf.damage.bands()),
                )
            })?,
            Output::Stream { format, sink, emit } => {
                pool.install(|| index.rebuild(&items, h));
                raster::render_bands_stream(
                    level,
                    &items,
                    clear,
                    (w, h),
                    emit,
                    *format,
                    sink.as_mut(),
                    Some(&peak),
                    has_backdrop,
                    index,
                    &mut frame_mem,
                )?
            }
        };
        surf.damage.commit();
        stats.phases.shade_seconds += shade_at.elapsed().as_secs_f64();
        surf.backdrop_capture_peak = peak.load(std::sync::atomic::Ordering::Relaxed);
        surf.clip_scratch_peak = frame_mem
            .clip_peak
            .load(std::sync::atomic::Ordering::Relaxed);
        tracing::debug!(target: "cherenkov_cpu::profile",
            lower_s = stats.phases.lower_seconds - phases.lower_seconds,
            glyph_s = stats.phases.glyph_seconds - phases.glyph_seconds,
            shade_s = stats.phases.shade_seconds - phases.shade_seconds,
            items = items.len(), glyphs = glyph_reqs,
            index_bytes = index.heap_bytes(),
            query_peak = frame_mem.query_peak.load(std::sync::atomic::Ordering::Relaxed),
            index_query_peak = frame_mem.peak.load(std::sync::atomic::Ordering::Relaxed),
            "raster phases");
        stats.draws += draws;
        stats.instances += edges;
        stats.passes += surf.rendered_bands();
        Ok(())
    }

    /// Lowers the tree into `items` for a raster of `size` — the surface,
    /// or with `local` a projective layer's local image — placing nested
    /// projective images from `projected`, then fills every glyph slot the
    /// items read. Returns the glyph request count; the filters and
    /// backdrop groups the items use are added to `used`, and the time of
    /// each step to `stats.phases` (`lower_seconds`, `glyph_seconds`).
    #[expect(
        clippy::too_many_arguments,
        reason = "one lowering's inputs travel together"
    )]
    fn lower_items(
        &mut self,
        sf: &cherenkov::SurfaceFrame<'_>,
        items: &mut Vec<Item>,
        size: (u32, u32),
        frame: cherenkov::FrameId,
        stats: &mut FrameStats,
        (local, projected): (
            Option<(LayerId, cherenkov::kurbo::Affine)>,
            FxHashMap<LayerId, projective::Placed>,
        ),
        used: &mut projective::Used,
    ) -> Result<usize, RenderError> {
        let lower_at = cherenkov::Instant::now();
        let id = sf.id;
        let glyph_reqs;
        let glyphs_rasterized;
        // Lowering borrows the layer caches; the surface borrow ends
        // before glyph resolution touches `self.fonts`/`self.glyph_cache`.
        let lowered = {
            let Some(surf) = self.surfaces.get_mut(&id) else {
                return Ok(0);
            };
            let mut caches = std::mem::take(&mut surf.layers);
            let mut lowering = Lowering::new(
                (items, &mut surf.anchor_scratch),
                size,
                Some(&mut self.filters),
                frame,
                &mut self.fonts,
                &self.bitmap_fonts,
                (
                    &mut self.bitmap_cache,
                    &mut self.geometry,
                    &mut self.edge_scratch,
                ),
                self.level,
                &self.pool,
            );
            lowering.project(local, projected);
            if local.is_none() {
                lowering.damage_layers = std::mem::take(&mut surf.damage.layers);
                lowering.damage_layers.clear();
                lowering.damage_rows = std::mem::take(&mut surf.damage.command_rows);
                lowering.damage_rows.clear();
            }
            let result = lowering.run(id, sf.tree, &mut caches, &self.images);
            if local.is_none() {
                surf.damage.layers = std::mem::take(&mut lowering.damage_layers);
                surf.damage.command_rows = std::mem::take(&mut lowering.damage_rows);
                if result.is_err() {
                    // Some command caches may already hold their new output.
                    // A retry must not treat those cache hits as submitted pixels.
                    surf.damage.forget();
                }
            }
            glyphs_rasterized = lowering.glyphs_rasterized;
            stats.glyphs_rasterized += glyphs_rasterized;
            stats.commands_lowered += lowering.commands_lowered;
            stats.layers_composed += lowering.layers_composed;
            glyph_reqs = std::mem::take(&mut lowering.glyphs);
            used.absorb(lowering.take_used_filters(), lowering.take_used_groups());
            surf.layers = caches;
            result
        };
        lowered?;
        if glyphs_rasterized > 0 {
            self.refresh_cache_budgets();
        }
        let glyph_at = cherenkov::Instant::now();
        stats.phases.lower_seconds += glyph_at.duration_since(lower_at).as_secs_f64();
        self.resolve_glyphs(&glyph_reqs)?;
        stats.phases.glyph_seconds += glyph_at.elapsed().as_secs_f64();
        Ok(glyph_reqs.len())
    }

    /// Realizes every visible projective layer's local image, innermost
    /// first, reusing a retained image whose content stamp, density,
    /// layout and image-replacement count match and whose filters are not
    /// animating.
    /// Returns the images placed directly in the surface, and the filters
    /// and groups the local images use.
    #[expect(
        clippy::too_many_lines,
        reason = "keeps plan lookup, realization and budget accounting together"
    )]
    fn realize_projective(
        &mut self,
        sf: &cherenkov::SurfaceFrame<'_>,
        frame: cherenkov::FrameId,
        stats: &mut FrameStats,
        index: &mut bins::FrameIndex,
        meter: &mut bins::FrameMeter,
    ) -> Result<(FxHashMap<LayerId, projective::Placed>, projective::Used), RenderError> {
        let mut used = projective::Used::default();
        let Some(size) = self.surfaces.get(&sf.id).map(|surf| surf.size) else {
            return Ok((FxHashMap::default(), used));
        };
        let limits = cherenkov::lowering::projective::Limits {
            max_dimension: MAX_SURFACE,
            max_bytes: self.image_budget,
        };
        self.retire_projective(sf);
        let replacements = self.image_replacements;
        let plans = cherenkov::lowering::projective::plan(sf.tree, size, limits)?;
        let mut placed: FxHashMap<Option<LayerId>, FxHashMap<LayerId, projective::Placed>> =
            FxHashMap::default();
        let mut required = 0_u64;
        for plan in plans {
            let Some(layout) = plan.image else { continue };
            let nested = placed.remove(&Some(plan.layer)).unwrap_or_default();
            let key =
                projective::Key::new(&layout, sf.tree.content_stamp(plan.layer), replacements);
            let filters = &self.filters;
            let hit = self.surfaces.get_mut(&sf.id).and_then(|surf| {
                surf.projective.get_mut(&plan.layer).and_then(|entries| {
                    entries.iter_mut().find(|entry| {
                        entry.key == key
                            && !entry.filters.iter().any(|f| filters.wants_redraw(*f))
                            && !entry
                                .groups
                                .iter()
                                .any(|g| filters.wants_redraw_group(sf.id, BackdropId::new(*g)))
                    })
                })
            });
            let image = if let Some(entry) = hit {
                entry.last_used = self.frame_count;
                used.filters.extend(entry.filters.iter().copied());
                used.groups.extend(entry.groups.iter().copied());
                Arc::clone(&entry.image)
            } else {
                let mut local_used = projective::Used::default();
                let mut items = Vec::new();
                self.lower_items(
                    sf,
                    &mut items,
                    layout.size,
                    frame,
                    stats,
                    (Some((plan.layer, layout.local_to_texel)), nested),
                    &mut local_used,
                )?;
                let shade_at = cherenkov::Instant::now();
                let (w, h) = (layout.size.0 as usize, layout.size.1 as usize);
                let mut base = vec![[0.0_f32; 4]; w * h];
                let has_backdrop = !local_used.groups.is_empty();
                let (draws, edges) = self.pool.install(|| {
                    raster::render_bands(
                        self.level,
                        &items,
                        index,
                        [0.0; 4],
                        &mut base,
                        w,
                        h,
                        None,
                        has_backdrop,
                        meter,
                        None,
                    )
                })?;
                stats.draws += draws;
                stats.instances += edges;
                stats.passes += u32::try_from(h.div_ceil(raster::BAND_H)).unwrap_or(u32::MAX);
                let image = Arc::new(projective::ProjectedImage::build(&base, layout.size));
                stats.phases.shade_seconds += shade_at.elapsed().as_secs_f64();
                stats.projective_realized += 1;
                let entry = projective::Entry {
                    key,
                    image: Arc::clone(&image),
                    filters: local_used.filters.iter().copied().collect(),
                    groups: local_used.groups.iter().copied().collect(),
                    last_used: self.frame_count,
                };
                used.absorb(local_used.filters, local_used.groups);
                if let Some(surf) = self.surfaces.get_mut(&sf.id) {
                    let entries = surf.projective.entry(plan.layer).or_default();
                    entries.retain(|e| e.key.density.to_bits() != entry.key.density.to_bits());
                    entries.push(entry);
                }
                image
            };
            required += image.bytes();
            if required > self.image_budget {
                return Err(RenderError::ProjectiveUnsupported {
                    layer: plan.layer,
                    reason: format!(
                        "the frame's projective images need {required} bytes; the CPU budget admits {}",
                        self.image_budget
                    ),
                });
            }
            stats.projective_composed += 1;
            placed.entry(plan.parent).or_default().insert(
                plan.layer,
                projective::Placed {
                    image,
                    inverse: layout.inverse,
                    bounds: layout.bounds,
                    to_parent: layout.to_parent,
                    density: layout.density,
                },
            );
        }
        Ok((placed.remove(&None).unwrap_or_default(), used))
    }

    /// Drops every retained image of surface `sf` no frame can compose
    /// again: its layer is gone or affine, its content stamp moved on, or
    /// an image was replaced since. A resource released under #199's
    /// deferred release is drawn by no installed content, so every image
    /// that read it is among these: none outlives the frame of its release.
    fn retire_projective(&mut self, sf: &cherenkov::SurfaceFrame<'_>) {
        let (tree, replacements) = (sf.tree, self.image_replacements);
        if let Some(surf) = self.surfaces.get_mut(&sf.id) {
            for (layer, entries) in &mut surf.projective {
                entries.retain(|entry| {
                    tree.projective_pose(*layer).is_some()
                        && entry
                            .key
                            .is_current(tree.content_stamp(*layer), replacements)
                });
            }
            surf.projective.retain(|_, entries| !entries.is_empty());
        }
    }

    /// Bytes of every retained projective image.
    fn projective_bytes(&self) -> u64 {
        self.surfaces
            .values()
            .flat_map(|surf| surf.projective.values().flatten())
            .map(|entry| entry.image.bytes())
            .sum()
    }

    /// Evicts least-recently-used projective images not used this frame
    /// until everything fits the CPU budget; images the frame used are
    /// required and never evicted.
    fn evict_projective(&mut self) {
        let others = self.images.values().map(|image| image.bytes()).sum::<u64>()
            + self.glyph_cache.bytes()
            + self.bitmap_cache.bytes();
        let mut total = self.projective_bytes() + others;
        if total <= self.image_budget {
            return;
        }
        let mut optional: Vec<(u64, SurfaceId, LayerId, f64)> = self
            .surfaces
            .iter()
            .flat_map(|(sid, surf)| {
                surf.projective.iter().flat_map(move |(layer, entries)| {
                    entries
                        .iter()
                        .map(move |e| (e.last_used, *sid, *layer, e.key.density))
                })
            })
            .filter(|(last, ..)| *last != self.frame_count)
            .collect();
        optional.sort_unstable_by_key(|(last, ..)| *last);
        for (_, sid, layer, density) in optional {
            if total <= self.image_budget {
                break;
            }
            let Some(entries) = self
                .surfaces
                .get_mut(&sid)
                .and_then(|surf| surf.projective.get_mut(&layer))
            else {
                continue;
            };
            entries.retain(|e| {
                let evict = e.key.density.to_bits() == density.to_bits();
                if evict {
                    total -= e.image.bytes();
                }
                !evict
            });
        }
    }

    /// Fills every glyph request's slot: cache hits resolve directly;
    /// misses are rasterized in parallel on the worker pool, then
    /// inserted into the cache under its byte budget.
    fn resolve_glyphs(&mut self, reqs: &[lower::GlyphReq]) -> Result<(), RenderError> {
        use rayon::prelude::*;
        let mut missing = Vec::new();
        for req in reqs {
            if let Some(mask) = self.glyph_cache.get(&req.key) {
                let _ = req.slot.set(mask);
            } else {
                missing.push(req);
            }
        }
        if missing.is_empty() {
            return Ok(());
        }
        let fonts = &self.fonts;
        let pool = &self.pool;
        let masks: Vec<(glyph::GlyphKey, std::sync::Arc<glyph::GlyphMask>)> =
            pool.install(|| {
                missing
                    .par_iter()
                    .map(|req| {
                        let font = fonts.get(&req.font).ok_or_else(|| {
                            RenderError::Font(format!("unregistered font {}", req.font))
                        })?;
                        let mask = std::sync::Arc::new(glyph::rasterize_mask(&font.data, req)?);
                        let _ = req.slot.set(mask.clone());
                        Ok((req.key, mask))
                    })
                    .collect::<Result<Vec<_>, RenderError>>()
            })?;
        self.glyph_cache.insert_batch(masks);
        Ok(())
    }
}
