//! The render side of the [`Raster`](crate::Raster) backend: sole owner
//! of the framebuffers and the worker pool, driven by the shared front
//! end's render loop.

mod account;
mod bitmap;
mod blend;
mod colr;
mod filter;
mod font;
mod glyph;
mod image;
mod mesh;
use image::CpuImage;
use std::sync::Arc;
mod lower;
mod paint;
mod prepared;
pub mod present;
mod projective;
mod raster;

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
    /// The largest live pixel-buffer bytes in any band that ran a
    /// backdrop capture last frame (window, isolation stack and capture
    /// buffers). 0 when no capture ran.
    backdrop_capture_peak: u64,
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
        match &self.output {
            Output::Stream { emit, .. } => (emit.capacity() * size_of::<[half::f16; 4]>()) as u64,
            _ => 0,
        }
    }
}

/// All render-thread state: the [`Raster`](crate::Raster) backend's
/// [`Renderer`] implementation.
pub struct RasterRenderer {
    pool: rayon::ThreadPool,
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
    /// Bumped on every image replacement: the one change to what a
    /// current projective image read that arrives without a tree edit.
    /// A removal only runs once no installed content draws the image
    /// (#199), so it does not count.
    image_replacements: u64,
    /// Counts rendered frames; the projective cache's recency clock.
    frame_count: u64,
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
        .num_threads(config.threads.unwrap_or(0))
        .thread_name(|i| format!("cherenkov-raster-{i}"));
    builder
        .build()
        .map(|pool| {
            let info = RasterInfo {
                threads: pool.current_num_threads(),
                simd: "scalar",
                cpu: cpu_model(),
            };
            (
                RasterRenderer {
                    pool,
                    surfaces: FxHashMap::default(),
                    filters: filter::Registry::default(),
                    fonts: FxHashMap::default(),
                    bitmap_fonts: FxHashMap::default(),
                    images: FxHashMap::default(),
                    image_budget: config.budget.cpu.0,
                    glyph_cache: glyph::GlyphCache::new(config.budget.cpu.0),
                    bitmap_cache: bitmap::BitmapCache::new(config.budget.cpu.0),
                    image_replacements: 0,
                    frame_count: 0,
                },
                info,
            )
        })
        .map_err(|e| EngineError::Backend(format!("rayon pool: {e}")))
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

impl Renderer for RasterRenderer {
    type Target = RasterTarget;
    type Font = font::PreparedFont;

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
                size,
                output,
                refresh,
                layers: FxHashMap::default(),
                filters: Vec::new(),
                groups: Vec::new(),
                backdrop_capture_peak: 0,
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
            // The decoded image is f32 RGBA: sixteen bytes a texel.
            max_texels: self.image_budget / 16,
        }
    }

    fn add_image(&mut self, id: ImageId, image: ImageUpload) -> Result<(), ResourceError> {
        let resident: u64 = self.images.values().map(|image| image.bytes()).sum();
        let required = u64::from(image.width)
            .checked_mul(u64::from(image.height))
            .and_then(|count| count.checked_mul(16));
        if required.is_none_or(|bytes| bytes > self.image_budget.saturating_sub(resident)) {
            return Err(ResourceError::Image("CPU image budget exhausted".into()));
        }
        let decoded = Arc::new(CpuImage::decode(&image)?);
        self.images.insert(id.raw(), decoded);
        self.refresh_cache_budgets();
        Ok(())
    }

    fn replace_image(&mut self, id: ImageId, image: ImageUpload) -> Result<(), ResourceError> {
        let current = self
            .images
            .get(&id.raw())
            .expect("replace targets a registered image");
        let resized = if (current.width, current.height) == (image.width, image.height) {
            CpuImage::validate(&image)?;
            None
        } else {
            let resident =
                self.images.values().map(|image| image.bytes()).sum::<u64>() - current.bytes();
            let required = u64::from(image.width)
                .checked_mul(u64::from(image.height))
                .and_then(|count| count.checked_mul(16));
            if required.is_none_or(|bytes| bytes > self.image_budget.saturating_sub(resident)) {
                return Err(ResourceError::Image("CPU image budget exhausted".into()));
            }
            Some(Arc::new(CpuImage::decode(&image)?))
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
                .overwrite(&image);
        }
        Ok(())
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
                .map(cherenkov::lowering::Content::into_picture),
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
                .map(cherenkov::lowering::Content::into_picture),
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
    ) -> Result<FrameRedraw, RenderError> {
        self.render_frame(frame, stats)
    }

    #[cfg(target_arch = "wasm32")]
    fn render(
        &mut self,
        frame: &Frame<'_>,
        stats: &mut FrameStats,
    ) -> impl core::future::Future<Output = Result<FrameRedraw, RenderError>> {
        core::future::ready(self.render_frame(frame, stats))
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
            .sum::<u64>();
        categories.glyphs = self.glyph_cache.bytes();
        categories.images = self.images.values().map(|image| image.bytes()).sum();
        categories.colr = self.fonts.values().map(font::Font::colr_bytes).sum();
        categories.bitmaps = self.bitmap_cache.bytes();
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
            backdrop_captures: cherenkov::Bytes(backdrop_captures),
            backdrop_capture_format: (backdrop_captures > 0).then_some("linear-f32"),
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
                self.render_surface(sf, frame.id, stats)?;
            }
        }
        let (used, used_groups) = self.filter_uses();
        self.evict_projective();
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
    /// Lowers and rasterizes one surface's frame.
    #[expect(
        clippy::many_single_char_names,
        reason = "w/h and r/g/b/a are the natural names"
    )]
    fn render_surface(
        &mut self,
        sf: &cherenkov::SurfaceFrame<'_>,
        frame: cherenkov::FrameId,
        stats: &mut FrameStats,
    ) -> Result<(), RenderError> {
        let profile = tracing::enabled!(target: "cherenkov_cpu::profile", tracing::Level::DEBUG);
        let start = profile.then(cherenkov::Instant::now);
        let id = sf.id;
        let size = match self.surfaces.get(&id) {
            Some(surf) => surf.size,
            None => return Ok(()),
        };
        let (projected, mut used) = self.realize_projective(sf, frame, stats)?;
        let mut items: Vec<Item> = Vec::new();
        let (glyph_reqs, lowered_at) = self.lower_items(
            sf,
            &mut items,
            size,
            frame,
            stats,
            (None, projected),
            &mut used,
        )?;
        if let Some(surf) = self.surfaces.get_mut(&id) {
            surf.filters = used.filters.into_iter().collect();
            surf.groups = used.groups.into_iter().collect();
        }
        let resolved_at = start.map(|_| cherenkov::Instant::now());
        let Some(surf) = self.surfaces.get_mut(&id) else {
            return Ok(());
        };
        let (w, h) = (surf.size.0 as usize, surf.size.1 as usize);
        let [r, g, b, a] = sf.clear.components;
        let clear = [r * a, g * a, b * a, a];
        let pool = &self.pool;
        let peak = std::sync::atomic::AtomicU64::new(0);
        let has_backdrop = !surf.groups.is_empty();
        let (draws, edges) = match &mut surf.output {
            Output::F32(fb) => pool.install(|| {
                raster::render_bands(&items, clear, fb, w, h, Some(&peak), has_backdrop)
            })?,
            Output::F16(out) => pool.install(|| {
                raster::render_bands_f16(&items, clear, out, w, h, Some(&peak), has_backdrop)
            })?,
            Output::Stream { format, sink, emit } => raster::render_bands_stream(
                &items,
                clear,
                (w, h),
                emit,
                *format,
                sink.as_mut(),
                Some(&peak),
                has_backdrop,
            )?,
        };
        surf.backdrop_capture_peak = peak.load(std::sync::atomic::Ordering::Relaxed);
        if let (Some(start), Some(lowered), Some(resolved)) = (start, lowered_at, resolved_at) {
            tracing::debug!(target: "cherenkov_cpu::profile",
                lower_ns = lowered.duration_since(start).as_nanos(),
                glyph_ns = resolved.duration_since(lowered).as_nanos(),
                shade_ns = resolved.elapsed().as_nanos(),
                items = items.len(), glyphs = glyph_reqs, "raster phases");
        }
        stats.draws += draws;
        stats.instances += edges;
        stats.passes += u32::try_from(h.div_ceil(raster::BAND_H)).unwrap_or(u32::MAX);
        Ok(())
    }

    /// Lowers the tree into `items` for a raster of `size` — the surface,
    /// or with `local` a projective layer's local image — placing nested
    /// projective images from `projected`, then fills every glyph slot the
    /// items read. Returns the glyph request count and the instant
    /// lowering finished, before glyph resolution; the filters and
    /// backdrop groups the items use are added to `used`.
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
    ) -> Result<(usize, Option<cherenkov::Instant>), RenderError> {
        let id = sf.id;
        let glyph_reqs;
        let glyphs_rasterized;
        // Lowering borrows the layer caches; the surface borrow ends
        // before glyph resolution touches `self.fonts`/`self.glyph_cache`.
        let lowered = {
            let Some(surf) = self.surfaces.get_mut(&id) else {
                return Ok((0, None));
            };
            let mut caches = std::mem::take(&mut surf.layers);
            let mut lowering = Lowering::new(
                items,
                size,
                Some(&mut self.filters),
                frame,
                &mut self.fonts,
                &self.bitmap_fonts,
                &mut self.bitmap_cache,
            );
            lowering.project(local, projected);
            let result = lowering.run(id, sf.tree, &mut caches, &self.images);
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
        let lowered_at = tracing::enabled!(target: "cherenkov_cpu::profile", tracing::Level::DEBUG)
            .then(cherenkov::Instant::now);
        self.resolve_glyphs(&glyph_reqs)?;
        Ok((glyph_reqs.len(), lowered_at))
    }

    /// Realizes every visible projective layer's local image, innermost
    /// first, reusing a retained image whose content stamp, density,
    /// layout and image-replacement count match and whose filters are not
    /// animating.
    /// Returns the images placed directly in the surface, and the filters
    /// and groups the local images use.
    fn realize_projective(
        &mut self,
        sf: &cherenkov::SurfaceFrame<'_>,
        frame: cherenkov::FrameId,
        stats: &mut FrameStats,
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
                let (w, h) = (layout.size.0 as usize, layout.size.1 as usize);
                let mut base = vec![[0.0_f32; 4]; w * h];
                let has_backdrop = !local_used.groups.is_empty();
                let (draws, edges) = self.pool.install(|| {
                    raster::render_bands(&items, [0.0; 4], &mut base, w, h, None, has_backdrop)
                })?;
                stats.draws += draws;
                stats.instances += edges;
                stats.passes += u32::try_from(h.div_ceil(raster::BAND_H)).unwrap_or(u32::MAX);
                let image = Arc::new(projective::ProjectedImage::build(&base, layout.size));
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
