//! The Cherenkov implementation of the recording boundary (water-rs/hydrolysis#205, H1).
//!
//! [`Recording`] keeps the fixed imperative API drawing code records through;
//! internally it is an ordered op list. The flush lowers each contiguous run
//! into a [`waterui_graphics::draw::Content`] the mounted engine layer shows, so engine
//! mounts and hierarchy persist across frames while only the content payload
//! is replaced.
//!
//! Ops name peniko types because that is the currency the call sites already
//! produce; conversion into engine resources — fonts and images — happens at
//! lower time through [`SceneResources`], which deduplicates registrations
//! across frames while the recordings that name them hold them.

use core::cell::RefCell;
use core::fmt;
use core::hash::Hash;
use kurbo::{Affine, Rect, Shape, Vec2};
use nami::{Computed, Signal as _};
use peniko::{BlendMode, Fill, FontData};
use rustc_hash::FxHashMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use waterui_graphics::draw::{
    BlendSpace, ColorStop, Content, Draw, EvenOdd, Extend, FontId, Glyph as EngineGlyph,
    GlyphStyle, Group, ImageId, ImagePattern, Interpolation, LayoutSize, LinearGradient, Paint,
    RadialGradient, Recorder, Sampling, Shadow, ShapeData, SweepGradient, WorkingColor,
};
use waterui_graphics::{
    FontSource, HeldResources, ImageColorSpace, ImageData, PlainId, RecordingResources, Registered,
    ResourceError, Rgba8,
};

/// One positioned glyph in a shaped run — the currency text shaping hands
/// the recording.
#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    /// Glyph identifier within its font.
    pub(crate) id: u32,
    /// X offset within the run.
    pub(crate) x: f32,
    /// Y offset within the run.
    pub y: f32,
}

/// A run of glyphs from one font that share every drawing attribute.
#[derive(Debug)]
#[cfg_attr(hydrolysis_hwui, expect(dead_code))]
pub struct GlyphRun<'a> {
    /// The font these glyphs are indexed in.
    pub(crate) font: &'a FontData,
    /// Em size in pixels.
    pub(crate) font_size: f32,
    /// Variable-font axis positions, normalized, as shaping produced them.
    pub(crate) normalized_coords: &'a [i16],
    /// Transform applied to the whole run.
    pub(crate) transform: Affine,
    /// Paint for the glyphs, and the "foreground colour" for colour fonts.
    pub(crate) brush: &'a peniko::Brush,
    /// Extra alpha multiplier applied to `brush`.
    pub(crate) brush_alpha: f32,
    /// Whether the glyphs are filled or stroked.
    pub(crate) style: peniko::StyleRef<'a>,
    /// The glyphs, in run order.
    pub(crate) glyphs: &'a [Glyph],
}

/// The opaque recording object drawing code builds. Internally an ordered op
/// list lowered into a Cherenkov [`Content`] at flush; nothing about the ops
/// reaches the drawing-facing API.
#[derive(Clone, Default)]
pub struct Recording {
    ops: Vec<Op>,
    /// Push scopes still open — the tracked-stack invariant the flush asserts.
    open_layers: u32,
}

// `Op` payloads are draw commands, not loggable state — the op count is the
// only meaningful signal.
impl fmt::Debug for Recording {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Recording")
            .field("ops", &self.ops.len())
            .field("open_layers", &self.open_layers)
            .finish()
    }
}

#[derive(Clone)]
enum Op {
    LiveRect {
        transform: Affine,
        rect: Computed<Rect>,
        color: Computed<WorkingColor>,
    },
    Fill {
        transform: Affine,
        brush: peniko::Brush,
        brush_transform: Option<Affine>,
        shape: ShapeData,
    },
    /// A fill whose paint is already an engine paint — the resolved-color and
    /// `Gradient`-view path, which never round-trips through peniko's sRGB
    /// (a `WorkingColor` is extended-range Display P3; peniko cannot carry it).
    FillPaint {
        transform: Affine,
        paint: Paint,
        shape: ShapeData,
    },
    StrokePaint {
        stroke: kurbo::Stroke,
        transform: Affine,
        paint: Paint,
        shape: ShapeData,
    },
    #[cfg_attr(hydrolysis_hwui, expect(dead_code))]
    Glyphs {
        font: FontData,
        font_size: f32,
        coords: Arc<[i16]>,
        transform: Affine,
        brush: peniko::Brush,
        brush_alpha: f32,
        style: peniko::Style,
        glyphs: Arc<[Glyph]>,
    },
    BlurredRoundedRect {
        transform: Affine,
        rect: Rect,
        color: WorkingColor,
        radius: f64,
        sigma: f64,
    },
    /// A shape's blurred silhouette in one colour — the shadow view's
    /// output, rasterized and cached by the engine rather than a
    /// renderer-owned pixmap cache.
    Shadow {
        transform: Affine,
        shape: ShapeData,
        color: WorkingColor,
        sigma: f64,
    },
    PushGroup {
        blend: BlendMode,
        opacity: f32,
        transform: Affine,
        clip: ShapeData,
    },
    Picture {
        transform: Affine,
        picture: waterui_graphics::draw::Picture,
    },
    /// A platform text layout, drawn through the retained lease the painter
    /// handed over — the HWUI seam (water-rs/waterui#1811).
    #[cfg(any(test, hydrolysis_hwui))]
    Text {
        resource: Arc<crate::hwui::text::TextLayoutResource>,
        transform: Affine,
        clip: Option<Rect>,
    },
    PopLayer,
}

/// Panics unless `image`'s blob is exactly `format.size_in_bytes(width,
/// height)` — the `width * height * bytes_per_pixel` contract the upload
/// enforces. Checked at this boundary — the first hydrolysis-owned point —
/// so a malformed `peniko::ImageData` fails fast with the expected and
/// actual byte counts instead of surfacing deep inside the engine.
pub fn assert_well_formed_image(image: &peniko::ImageData) {
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

/// `image`'s RGBA8 bytes fitted to `limits`: an admitted source passes
/// through with its own pixels; a larger one — a decoded photo past the
/// device's texture limit or the whole image budget — is
/// Lanczos3-resampled to the largest admitted size at the same aspect
/// ratio, so the registration below still holds the engine's contract
/// rather than panicking on it. Returns `(width, height, bytes)`.
fn fit_image_to_limits(
    image: &peniko::ImageData,
    limits: waterui_graphics::draw::ImageLimits,
) -> (u32, u32, Arc<[u8]>) {
    let (width, height) = limits.fit::<waterui_graphics::draw::Rgba8>(image.width, image.height);
    assert!(
        (width, height) != (0, 0),
        "hydrolysis renderer: engine image limits admit no image at all"
    );
    if (width, height) == (image.width, image.height) {
        return (width, height, Arc::from(image.data.data()));
    }
    let source = image::RgbaImage::from_raw(image.width, image.height, image.data.data().to_vec())
        .expect("hydrolysis renderer: a well-formed image fits RgbaImage::from_raw");
    let resized = image::imageops::resize(
        &source,
        width,
        height,
        image::imageops::FilterType::Lanczos3,
    );
    (width, height, Arc::from(resized.into_raw()))
}

/// Engine-facing caches shared by every lowered scene: registered fonts and
/// images, keyed by the identity of the peniko resource they came from.
///
/// Registration itself goes through the one `SceneResources` table
/// (`waterui_graphics::resources::SceneResources`, the #1324 contract) the
/// host built over its `Engine`. The table alone already dedups by content
/// hash — the identity key here keeps that hash, the copy of a peniko blob
/// into the table's source bytes, and the engine upload behind a cache miss,
/// out of the per-frame lowering path.
///
/// The caches own nothing: like the table, they list each entry weakly, and
/// the recordings that name an entry hold it through their
/// [`LoweredResources`]. An entry therefore lives exactly as long as some
/// recording that names it may still be drawn; when the last such set drops,
/// the entry leaves its cache and lets go of its registration.
pub struct SceneResources {
    /// The `SceneContent::build_scene` resource table, shared with every scene
    /// view this engine draws.
    inner: waterui_graphics::SceneResources,
    fonts: IdentityCache<FontKey, FontId>,
    images: IdentityCache<ImageKey, ImageId>,
    /// Fresh engine registrations since the last
    /// [`Self::take_registration_stats`] — the frame-work counters' evidence
    /// that fonts and images are not re-registered per frame.
    font_registrations: std::cell::Cell<u64>,
    image_registrations: std::cell::Cell<u64>,
}

/// What identifies a peniko resource in an identity cache.
trait CacheKey: Copy + Eq + Hash {}

impl<K: Copy + Eq + Hash> CacheKey for K {}

/// The live entries of one identity cache, each listed weakly so that the
/// listing never keeps an entry alive.
type Listing<K, I> = RefCell<FxHashMap<K, Weak<Cached<K, I>>>>;

/// One identity cache: a peniko resource's identity to the registration it
/// lowered to, for as long as some recording names it.
struct IdentityCache<K: CacheKey, I: PlainId> {
    listing: Rc<Listing<K, I>>,
}

impl<K: CacheKey, I: PlainId> IdentityCache<K, I> {
    fn new() -> Self {
        Self {
            listing: Rc::new(RefCell::new(FxHashMap::default())),
        }
    }

    /// The live entry for `key`, if a recording still names it.
    fn get(&self, key: K) -> Option<Rc<Cached<K, I>>> {
        self.listing.borrow().get(&key).and_then(Weak::upgrade)
    }

    /// Lists `registered` under `key`, returning the entry the naming
    /// recording holds.
    fn insert(&self, key: K, registered: Registered<I>) -> Rc<Cached<K, I>> {
        let entry = Rc::new(Cached {
            registered,
            key,
            listing: Rc::downgrade(&self.listing),
        });
        self.listing.borrow_mut().insert(key, Rc::downgrade(&entry));
        entry
    }

    /// How many entries are listed.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.listing.borrow().len()
    }
}

/// A cached registration. The recordings that name it hold it; when the
/// last of them lets go it leaves its cache and drops its share of the
/// registration, which the table then releases once nothing else holds it.
struct Cached<K: CacheKey, I: PlainId> {
    registered: Registered<I>,
    key: K,
    listing: Weak<Listing<K, I>>,
}

impl<K: CacheKey, I: PlainId> Drop for Cached<K, I> {
    fn drop(&mut self) {
        // The listing's weak entry stops upgrading the moment the last
        // strong reference goes, and this runs in that same moment, so the
        // entry under `key` is this one: no newer entry can have replaced it.
        if let Some(listing) = self.listing.upgrade() {
            listing.borrow_mut().remove(&self.key);
        }
    }
}

/// The resource side of one recording as Hydrolysis lowers it: the shared
/// table's naming, plus the identity-cache entries the recording names.
pub struct LoweringNames<'a> {
    table: RecordingResources<'a>,
    fonts: FxHashMap<FontKey, Rc<Cached<FontKey, FontId>>>,
    images: FxHashMap<ImageKey, Rc<Cached<ImageKey, ImageId>>>,
    /// Text-layout leases this recording's `Text` ops name.
    #[cfg(any(test, hydrolysis_hwui))]
    text_layouts:
        FxHashMap<waterui_graphics::draw::TextLayoutId, Arc<crate::hwui::text::TextLayoutResource>>,
}

impl LoweringNames<'_> {
    /// The resources this recording names, for the mount to keep beside it.
    fn finish(self) -> LoweredResources {
        LoweredResources {
            held: self.table.finish(),
            fonts: self.fonts.into_values().collect(),
            images: self.images.into_values().collect(),
            #[cfg(any(test, hydrolysis_hwui))]
            text_layouts: self.text_layouts.into_values().collect(),
        }
    }
}

/// Everything one lowered recording names: its registrations in the shared
/// table and its entries in Hydrolysis's identity caches.
///
/// The mount keeps it for as long as the recording is installed and lets it
/// go only once a recording that replaces it has been installed — the
/// [`HeldResources`] contract, which this extends to the identity caches.
pub struct LoweredResources {
    held: HeldResources,
    fonts: Box<[Rc<Cached<FontKey, FontId>>]>,
    images: Box<[Rc<Cached<ImageKey, ImageId>>]>,
    /// The text-layout leases installed content still names: the layout
    /// lives until the last installed recording holding it is replaced.
    #[cfg(any(test, hydrolysis_hwui))]
    text_layouts: Box<[Arc<crate::hwui::text::TextLayoutResource>]>,
}

impl fmt::Debug for LoweredResources {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("LoweredResources");
        d.field("held", &self.held)
            .field("fonts", &self.fonts.len())
            .field("images", &self.images.len());
        #[cfg(any(test, hydrolysis_hwui))]
        d.field("text_layouts", &self.text_layouts.len());
        d.finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FontKey {
    blob: u64,
    index: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ImageKey {
    blob: u64,
    width: u32,
    height: u32,
    format: u8,
    alpha: u8,
}

impl fmt::Debug for SceneResources {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SceneResources")
            .field("fonts", &self.fonts.listing.borrow().len())
            .field("images", &self.images.listing.borrow().len())
            .finish_non_exhaustive()
    }
}

impl SceneResources {
    /// Creates caches served by `engine`.
    #[cfg(hydrolysis_gpu_painter)]
    pub(crate) fn new(engine: &Rc<crate::engine::GpuEngine>) -> Self {
        Self::over(waterui_graphics::SceneResources::with_shaders(
            engine.clone(),
            engine.clone(),
        ))
    }

    /// Creates caches over the registration table `inner`.
    pub(crate) fn over(inner: waterui_graphics::SceneResources) -> Self {
        Self {
            inner,
            fonts: IdentityCache::new(),
            images: IdentityCache::new(),
            font_registrations: std::cell::Cell::new(0),
            image_registrations: std::cell::Cell::new(0),
        }
    }

    /// Begins the resource side of one lowered recording.
    fn recording(&self) -> LoweringNames<'_> {
        LoweringNames {
            table: self.inner.recording(),
            #[cfg(any(test, hydrolysis_hwui))]
            text_layouts: FxHashMap::default(),
            fonts: FxHashMap::default(),
            images: FxHashMap::default(),
        }
    }

    /// How many (fonts, images) the identity caches list — each one named
    /// by a recording that may still be drawn.
    #[cfg(test)]
    pub(crate) fn cached(&self) -> (usize, usize) {
        (self.fonts.len(), self.images.len())
    }

    /// (fonts, images) registered since the last call, drained per frame
    /// into the frame-work counters.
    pub(crate) fn take_registration_stats(&self) -> (u64, u64) {
        let stats = (
            self.font_registrations.get(),
            self.image_registrations.get(),
        );
        self.font_registrations.set(0);
        self.image_registrations.set(0);
        stats
    }

    /// The table `SceneContent::build_scene` calls register through.
    pub(crate) const fn waterui(&self) -> &waterui_graphics::SceneResources {
        &self.inner
    }

    /// The registered font id for `font`, uploading the face on first use and
    /// holding the registration for `names`' recording.
    pub(crate) fn font(&self, names: &mut LoweringNames<'_>, font: &FontData) -> FontId {
        let key = FontKey {
            blob: font.data.id(),
            index: font.index,
        };
        if let Some(cached) = names.fonts.get(&key) {
            return names.table.name(&cached.registered);
        }
        let cached = self.fonts.get(key).unwrap_or_else(|| {
            let source =
                FontSource::bytes(Arc::<[u8]>::from(font.data.data())).with_index(font.index);
            let registered = self
                .inner
                .font(source)
                .expect("hydrolysis renderer: engine rejected a font source it must accept");
            self.font_registrations
                .set(self.font_registrations.get() + 1);
            self.fonts.insert(key, registered)
        });
        let id = names.table.name(&cached.registered);
        names.fonts.insert(key, cached);
        id
    }

    /// The registered image id for `image`, uploading the pixels on first use
    /// and holding the registration for `names`' recording.
    ///
    /// # Errors
    ///
    /// The engine's [`ResourceError`] when it does not register the image —
    /// [`ResourceError::OverBudget`] when the image does not fit the image
    /// budget beside the images already registered.
    pub(crate) fn image(
        &self,
        names: &mut LoweringNames<'_>,
        image: &peniko::ImageData,
    ) -> Result<ImageId, ResourceError> {
        let key = ImageKey {
            blob: image.data.id(),
            width: image.width,
            height: image.height,
            format: image.format as u8,
            alpha: image.alpha_type as u8,
        };
        if let Some(cached) = names.images.get(&key) {
            return Ok(names.table.name(&cached.registered));
        }
        let cached = if let Some(cached) = self.images.get(key) {
            cached
        } else {
            assert_eq!(
                image.format,
                peniko::ImageFormat::Rgba8,
                "hydrolysis renderer: unsupported image format {:?}; only Rgba8 uploads reach the engine",
                image.format
            );
            assert_well_formed_image(image);
            // A decoded image can exceed what the engine holds: resample to
            // the limits' fit so the registration is the contract it asserts.
            let (width, height, bytes) = fit_image_to_limits(image, self.inner.image_limits());
            let data = ImageData::<Rgba8>::new(width, height, bytes)
                .expect("hydrolysis renderer: well-formed Rgba8 image rejected")
                .color_space(ImageColorSpace::Srgb);
            let data = if image.alpha_type == peniko::ImageAlphaType::AlphaPremultiplied {
                data.premultiplied()
            } else {
                data
            };
            // A refused registration caches nothing: a later recording
            // naming the image tries a registration of its own.
            let registered = self.inner.image(data)?;
            self.image_registrations
                .set(self.image_registrations.get() + 1);
            self.images.insert(key, registered)
        };
        let id = names.table.name(&cached.registered);
        names.images.insert(key, cached);
        Ok(id)
    }

    /// The engine paint for `brush`, registering any image it names and
    /// holding it for `names`' recording.
    ///
    /// # Errors
    ///
    /// The engine's [`ResourceError`] when it does not register the
    /// brush's image; see [`image`](Self::image).
    pub(crate) fn paint(
        &self,
        names: &mut LoweringNames<'_>,
        brush: &peniko::Brush,
    ) -> Result<Paint, ResourceError> {
        Ok(match brush {
            peniko::Brush::Solid(color) => Paint::Solid(working_color(*color)),
            peniko::Brush::Gradient(gradient) => gradient_paint(gradient),
            peniko::Brush::Image(brush) => {
                let pattern = ImagePattern {
                    image: self.image(names, &brush.image)?,
                    transform: Affine::IDENTITY,
                    extend_x: extend(brush.sampler.x_extend),
                    extend_y: extend(brush.sampler.y_extend),
                    sampling: sampling(brush.sampler.quality),
                };
                Paint::Image(pattern)
            }
        })
    }

    /// `brush` mapped through its own coordinate transform.
    ///
    /// # Errors
    ///
    /// The engine's [`ResourceError`] when it does not register the
    /// brush's image; see [`image`](Self::image).
    pub(crate) fn transformed_paint(
        &self,
        names: &mut LoweringNames<'_>,
        brush: &peniko::Brush,
        brush_transform: Option<Affine>,
    ) -> Result<Paint, ResourceError> {
        Ok(transform_paint(self.paint(names, brush)?, brush_transform))
    }
}

impl Recording {
    /// Creates an empty recording.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Whether nothing was recorded.
    pub(crate) const fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Clears all recorded commands.
    #[cfg(test)]
    pub(crate) fn reset(&mut self) {
        self.ops.clear();
        self.open_layers = 0;
    }

    /// Fills `shape` under `transform` with `brush`.
    pub(crate) fn fill<S: Shape + 'static>(
        &mut self,
        rule: Fill,
        transform: Affine,
        brush: &peniko::Brush,
        brush_transform: Option<Affine>,
        shape: &S,
    ) {
        self.ops.push(Op::Fill {
            transform,
            brush: brush.clone(),
            brush_transform,
            shape: shape_data(rule, shape),
        });
    }

    /// Records a rectangle whose geometry and colour remain live engine operands.
    pub(crate) fn live_rect(
        &mut self,
        transform: Affine,
        rect: Computed<Rect>,
        color: Computed<WorkingColor>,
    ) {
        self.ops.push(Op::LiveRect {
            transform,
            rect,
            color,
        });
    }

    /// Fills `shape` under `transform` with an engine paint.
    pub(crate) fn fill_paint<S: Shape + 'static>(
        &mut self,
        rule: Fill,
        transform: Affine,
        paint: Paint,
        shape: &S,
    ) {
        self.ops.push(Op::FillPaint {
            transform,
            paint,
            shape: shape_data(rule, shape),
        });
    }

    /// Strokes `shape` under `transform` with an engine paint.
    pub(crate) fn stroke_paint<S: Shape + 'static>(
        &mut self,
        stroke: &kurbo::Stroke,
        transform: Affine,
        paint: Paint,
        shape: &S,
    ) {
        self.ops.push(Op::StrokePaint {
            stroke: stroke.clone(),
            transform,
            paint,
            shape: shape_data(Fill::NonZero, shape),
        });
    }

    /// Draws a run of shaped glyphs.
    #[cfg_attr(hydrolysis_hwui, expect(dead_code))]
    pub(crate) fn glyphs(&mut self, run: &GlyphRun<'_>) {
        if run.glyphs.is_empty() {
            return;
        }
        self.ops.push(Op::Glyphs {
            font: run.font.clone(),
            font_size: run.font_size,
            coords: Arc::from(run.normalized_coords),
            transform: run.transform,
            brush: run.brush.clone(),
            brush_alpha: run.brush_alpha,
            style: run.style.to_owned(),
            glyphs: Arc::from(run.glyphs),
        });
    }

    /// Draws a blurred rounded rect shadow under `transform`.
    pub(crate) fn blurred_rounded_rect(
        &mut self,
        transform: Affine,
        rect: Rect,
        color: WorkingColor,
        corner_radius: f64,
        sigma: f64,
    ) {
        self.ops.push(Op::BlurredRoundedRect {
            transform,
            rect,
            color,
            radius: corner_radius,
            sigma,
        });
    }

    /// Draws a recorded picture under `transform` — the content of a child
    /// view recorded against the same engine.
    pub(crate) fn draw_picture(
        &mut self,
        transform: Affine,
        picture: waterui_graphics::draw::Picture,
    ) {
        self.ops.push(Op::Picture { transform, picture });
    }

    /// Draws the platform text layout `resource` leases, under `transform`
    /// and clipped to `clip` in layout-local coordinates.
    #[cfg(any(test, hydrolysis_hwui))]
    pub(crate) fn draw_platform_text(
        &mut self,
        resource: Arc<crate::hwui::text::TextLayoutResource>,
        transform: Affine,
        clip: Option<Rect>,
    ) {
        self.ops.push(Op::Text {
            resource,
            transform,
            clip,
        });
    }

    /// Draws `shape`'s Gaussian-blurred silhouette in `color` under
    /// `transform` — `sigma` in the shape's units.
    pub(crate) fn shadow<S: Shape + 'static>(
        &mut self,
        transform: Affine,
        shape: &S,
        sigma: f64,
        color: WorkingColor,
    ) {
        self.ops.push(Op::Shadow {
            transform,
            shape: shape_data(Fill::NonZero, shape),
            color,
            sigma,
        });
    }

    /// Pushes a compositing scope with blend mode, opacity and clip.
    pub(crate) fn push_group<S: Shape + 'static>(
        &mut self,
        rule: Fill,
        blend: BlendMode,
        opacity: f32,
        clip_transform: Affine,
        clip: &S,
    ) {
        self.open_layers += 1;
        self.ops.push(Op::PushGroup {
            blend,
            opacity,
            transform: clip_transform,
            clip: shape_data(rule, clip),
        });
    }

    /// Opens a compositing scope, runs `f` inside it, then closes it — the
    /// lexical pairing recording call sites use when their content is not a
    /// whole renderer traversal (`&mut Recording`, not `&mut Renderer`).
    pub(crate) fn with_group<S: Shape + 'static>(
        &mut self,
        rule: Fill,
        blend: BlendMode,
        opacity: f32,
        clip_transform: Affine,
        clip: &S,
        f: impl FnOnce(&mut Self),
    ) {
        self.push_group(rule, blend, opacity, clip_transform, clip);
        f(self);
        self.pop_scope();
    }

    /// Closes the current scope.
    pub(crate) fn pop_scope(&mut self) {
        assert!(
            self.open_layers > 0,
            "hydrolysis recording: layer pop without a matching push"
        );
        self.open_layers -= 1;
        self.ops.push(Op::PopLayer);
    }

    /// Appends `other` under `placement`.
    pub(crate) fn append(&mut self, other: &Self, placement: Affine) {
        self.ops.extend(other.ops.iter().cloned().map(|mut op| {
            match &mut op {
                Op::LiveRect { transform, .. }
                | Op::Fill { transform, .. }
                | Op::FillPaint { transform, .. }
                | Op::StrokePaint { transform, .. }
                | Op::Glyphs { transform, .. }
                | Op::BlurredRoundedRect { transform, .. }
                | Op::Shadow { transform, .. }
                | Op::PushGroup { transform, .. }
                | Op::Picture { transform, .. } => *transform = placement * *transform,
                #[cfg(any(test, hydrolysis_hwui))]
                Op::Text { transform, .. } => *transform = placement * *transform,
                Op::PopLayer => {}
            }
            op
        }));
    }

    /// Each clip/opacity scope's clip rect with the transform it was pushed
    /// under, in op order — `transform` maps the clip's own rect into scene
    /// space, so `transform * clip` is the rect the scope clips to. For
    /// tests asserting clip geometry.
    #[cfg(test)]
    pub(crate) fn clip_scopes(&self) -> impl Iterator<Item = (Affine, Rect)> {
        self.ops.iter().filter_map(|op| match op {
            Op::PushGroup {
                transform, clip, ..
            } => Some((*transform, clip.bounds())),
            _ => None,
        })
    }

    /// Opens a plain clip scope of `clip` under `transform` — for tests
    /// flattening committed layer clips back into one recording.
    #[cfg(test)]
    pub(crate) fn push_clip_data(&mut self, transform: Affine, clip: ShapeData) {
        self.open_layers += 1;
        self.ops.push(Op::PushGroup {
            blend: BlendMode::default(),
            opacity: 1.0,
            transform,
            clip,
        });
    }

    /// Each fill op's shape bounds with the transform it was drawn under,
    /// in op order — `transform * shape` is the rect the fill paints. For
    /// tests asserting paint geometry.
    #[cfg(test)]
    pub(crate) fn fill_bounds(&self) -> impl Iterator<Item = (Affine, Rect)> {
        self.ops.iter().filter_map(|op| match op {
            Op::LiveRect {
                transform, rect, ..
            } => Some((*transform, rect.snapshot())),
            Op::Fill {
                transform, shape, ..
            }
            | Op::FillPaint {
                transform, shape, ..
            } => Some((*transform, shape.bounds())),
            _ => None,
        })
    }

    /// Each recorded glyph run's transform and run-local glyph offsets, in op
    /// order — the actual positions a flush lowers. `append` has already folded
    /// every placement into the op's transform, so `transform * (x, y)` is the
    /// point the ink lands at. For tests asserting where text paints.
    #[cfg(test)]
    pub(crate) fn glyph_runs(&self) -> impl Iterator<Item = (Affine, &[Glyph])> {
        self.ops.iter().filter_map(|op| match op {
            Op::Glyphs {
                transform, glyphs, ..
            } => Some((*transform, glyphs.as_ref())),
            _ => None,
        })
    }

    /// The premultiplied sRGB colours of every solid fill or stroke op, for
    /// tests asserting a colour reached the recording.
    #[cfg(test)]
    pub(crate) fn solid_fill_colours(&self) -> Vec<u32> {
        self.ops
            .iter()
            .filter_map(|op| match op {
                Op::Fill {
                    brush: peniko::Brush::Solid(color),
                    ..
                } => Some(u32::from_ne_bytes(color.to_rgba8().to_u8_array())),
                _ => None,
            })
            .collect()
    }

    /// Lowers the recording into engine [`Content`] through `resources`,
    /// returning the set of registrations the content names beside it.
    ///
    /// The caller keeps the [`LoweredResources`] for as long as the content
    /// is installed on a layer — the mounts own that lifetime, releasing a
    /// set only once the replacement content is installed.
    ///
    /// # Panics
    /// Panics when a layer push has no matching pop — the flush keeps the
    /// invariant on every code path it accepts.
    pub(crate) fn record_on(
        &self,
        recorder: &mut Recorder,
        resources: &SceneResources,
    ) -> LoweredResources {
        let mut names = resources.recording();
        let mut index = 0;
        lower(&self.ops, &mut index, recorder, resources, &mut names);
        assert_eq!(
            index,
            self.ops.len(),
            "hydrolysis recording: layer pop without a matching push"
        );
        names.finish()
    }

    /// The axis-aligned rectangles this recording fills with a single
    /// colour, in paint order and in the recording's own space, each
    /// already cut to the rectangular clips around it and carrying the
    /// alpha of the opacity scopes it sits in.
    ///
    /// A fill counts only when what reaches the target is exactly that
    /// colour over that rectangle: a solid paint, an axis-aligned
    /// transform, a plain rectangle, and every enclosing scope a normal
    /// blend under an axis-aligned rectangular clip. Everything else —
    /// gradients, images, glyphs, strokes, shadows, pictures, rounded or
    /// rotated shapes, and any fill inside a scope that fails the test —
    /// is left out: the summary names the flat colour fields a recording
    /// lays down, which is what a host matching its own chrome to the
    /// content's edge needs ([`SolidRect`]).
    pub(crate) fn solid_rects(&self) -> Box<[SolidRect]> {
        /// An open scope: the clip it narrows to and its opacity, or
        /// `None` when nothing inside it can be summarised.
        type Scope = Option<(Rect, f32)>;
        let mut scopes: Vec<Scope> = Vec::new();
        let mut rects = Vec::new();
        for op in &self.ops {
            let (transform, color, rect) = match op {
                Op::Fill {
                    transform,
                    brush: peniko::Brush::Solid(color),
                    shape: ShapeData::Rect(rect),
                    ..
                } => (
                    transform,
                    Computed::constant(working_color(*color)),
                    Computed::constant(*rect),
                ),
                Op::FillPaint {
                    transform,
                    paint: Paint::Solid(color),
                    shape: ShapeData::Rect(rect),
                } => (
                    transform,
                    Computed::constant(*color),
                    Computed::constant(*rect),
                ),
                Op::LiveRect {
                    transform,
                    rect,
                    color,
                } => (transform, color.clone(), rect.clone()),
                Op::PushGroup {
                    blend,
                    opacity,
                    transform,
                    clip,
                    ..
                } => {
                    let parent = scopes.last().copied().unwrap_or(Some((UNBOUNDED, 1.0)));
                    let summarisable = *blend == BlendMode::default()
                        && axis_aligned(*transform)
                        && matches!(clip, ShapeData::Rect(_));
                    scopes.push(match (parent, clip) {
                        (Some((bounds, alpha)), ShapeData::Rect(clip)) if summarisable => Some((
                            bounds.intersect(transform.transform_rect_bbox(*clip)),
                            alpha * opacity,
                        )),
                        _ => None,
                    });
                    continue;
                }
                Op::PopLayer => {
                    scopes.pop();
                    continue;
                }
                _ => continue,
            };
            let Some((bounds, alpha)) = scopes.last().copied().unwrap_or(Some((UNBOUNDED, 1.0)))
            else {
                continue;
            };
            if !axis_aligned(*transform) {
                continue;
            }
            rects.push(SolidRect {
                rect,
                color,
                transform: *transform,
                clip: bounds,
                alpha,
            });
        }
        rects.into_boxed_slice()
    }
}

/// A rectangle a recording fills with one colour: its bounds in the space
/// the recording was made in, and the colour that reaches the target there,
/// straight alpha folded with every enclosing opacity scope.
#[derive(Clone, Debug)]
pub struct SolidRect {
    rect: Computed<Rect>,
    color: Computed<WorkingColor>,
    transform: Affine,
    clip: Rect,
    alpha: f32,
}

impl SolidRect {
    /// Samples the current clipped rectangle and its opacity-adjusted colour.
    #[must_use]
    pub fn current(&self) -> (Rect, WorkingColor) {
        let rect = self
            .clip
            .intersect(self.transform.transform_rect_bbox(self.rect.snapshot()));
        let [r, g, b, a] = self.color.snapshot().components;
        (rect, WorkingColor::new([r, g, b, a * self.alpha]))
    }
}

/// The bounds of a fill no clip narrows.
const UNBOUNDED: Rect = Rect::new(
    f64::NEG_INFINITY,
    f64::NEG_INFINITY,
    f64::INFINITY,
    f64::INFINITY,
);

/// Whether `transform` keeps an axis-aligned rectangle axis-aligned and
/// unflipped — scale and translation only — so its bounding box is the
/// exact image of the rectangle.
pub fn axis_aligned(transform: Affine) -> bool {
    let [a, b, c, d, _, _] = transform.as_coeffs();
    b == 0.0 && c == 0.0 && a > 0.0 && d > 0.0
}

/// The paint of an op, or `None` when the engine did not register the
/// image its brush names: that op is not drawn, and the rejection is
/// reported as a warning naming the engine's reason, instead of taking the
/// window down. Nothing stands in for the image.
fn drawable(paint: Result<Paint, ResourceError>) -> Option<Paint> {
    paint
        .inspect_err(|error| {
            tracing::warn!(%error, "hydrolysis renderer: the engine did not register an image brush; the op that paints with it is not drawn");
        })
        .ok()
}

#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
fn lower(
    ops: &[Op],
    index: &mut usize,
    recorder: &mut Recorder,
    resources: &SceneResources,
    names: &mut LoweringNames<'_>,
) {
    while *index < ops.len() {
        let op = &ops[*index];
        *index += 1;
        match op {
            Op::LiveRect {
                transform,
                rect,
                color,
            } => {
                recorder.transform(*transform, |r| r.fill(rect.clone(), color.clone()));
            }
            Op::Fill {
                transform,
                brush,
                brush_transform,
                shape,
                ..
            } => {
                let Some(paint) =
                    drawable(resources.transformed_paint(names, brush, *brush_transform))
                else {
                    continue;
                };
                recorder.transform(*transform, |r| r.fill(shape.clone(), paint));
            }
            Op::FillPaint {
                transform,
                paint,
                shape,
            } => {
                recorder.transform(*transform, |r| r.fill(shape.clone(), paint.clone()));
            }
            Op::StrokePaint {
                stroke,
                transform,
                paint,
                shape,
            } => {
                recorder.transform(*transform, |r| {
                    r.stroke(shape.clone(), stroke.clone(), paint.clone());
                });
            }
            Op::Glyphs {
                font,
                font_size,
                coords,
                transform,
                brush,
                brush_alpha,
                style,
                glyphs,
            } => {
                let font = resources.font(names, font);
                let Some(mut paint) = drawable(resources.paint(names, brush)) else {
                    continue;
                };
                if *brush_alpha < 1.0 {
                    paint = alpha_scaled_paint(paint, *brush_alpha);
                }
                let style = match style {
                    peniko::Style::Fill(_) => GlyphStyle::Fill,
                    peniko::Style::Stroke(stroke) => GlyphStyle::Stroke(stroke.clone()),
                };
                let run = waterui_graphics::draw::GlyphRun {
                    font,
                    size: *font_size,
                    coords: coords.to_vec().into(),
                    glyphs: glyphs
                        .iter()
                        .map(|glyph| EngineGlyph {
                            id: glyph.id,
                            x: glyph.x,
                            y: glyph.y,
                            transform: None,
                        })
                        .collect(),
                    style,
                };
                recorder.transform(*transform, |r| r.glyphs(run, paint));
            }
            Op::BlurredRoundedRect {
                transform,
                rect,
                color,
                radius,
                sigma,
            } => {
                let shape = ShapeData::of(&rect.to_rounded_rect(*radius));
                let shadow = Shadow {
                    sigma: *sigma,
                    offset: Vec2::ZERO,
                    spread: 0.0,
                    color: *color,
                };
                recorder.transform(*transform, |r| r.shadow(shape, shadow));
            }
            Op::Shadow {
                transform,
                shape,
                color,
                sigma,
            } => {
                let shadow = Shadow {
                    sigma: *sigma,
                    offset: Vec2::ZERO,
                    spread: 0.0,
                    color: *color,
                };
                recorder.transform(*transform, |r| r.shadow(shape.clone(), shadow));
            }
            Op::PushGroup {
                transform,
                clip,
                blend,
                opacity,
                ..
            } => {
                let blend = blend_mode(*blend);
                let alpha = *opacity;
                recorder.transform(*transform, |r| {
                    r.clip(clip.clone(), |r| {
                        // The ops inside the scope carry the ambient transform
                        // they were recorded under — the clip's own transform
                        // belongs to the clip shape only, so the body replays
                        // under the push site's ambient, restored inside the
                        // scope to keep it from compounding onto every op.
                        r.transform(transform.inverse(), |r| {
                            if alpha >= 1.0 && blend == waterui_graphics::draw::BlendMode::Normal {
                                lower(ops, index, r, resources, names);
                            } else {
                                let group = Group {
                                    opacity: alpha,
                                    blend,
                                    blend_space: BlendSpace::Linear,
                                    filter: None,
                                };
                                r.group(group, |r| lower(ops, index, r, resources, names));
                            }
                        });
                    });
                });
            }
            Op::Picture { transform, picture } => recorder.picture(picture, *transform),
            #[cfg(any(test, hydrolysis_hwui))]
            Op::Text {
                resource,
                transform,
                clip,
            } => {
                let id = resource.id();
                names
                    .text_layouts
                    .entry(id)
                    .or_insert_with(|| Arc::clone(resource));
                if let Some(clip) = clip {
                    recorder.transform(*transform, |r| {
                        r.clip(*clip, |r| r.text(id, Affine::IDENTITY));
                    });
                } else {
                    recorder.text(id, *transform);
                }
            }
            Op::PopLayer => return,
        }
    }
}

/// An owned shape carrying `rule`'s winding.
fn shape_data<S: Shape + 'static>(rule: Fill, shape: &S) -> ShapeData {
    match rule {
        Fill::NonZero => ShapeData::of(shape),
        Fill::EvenOdd => ShapeData::of(&EvenOdd(
            shape.to_path(waterui_graphics::draw::PATH_TOLERANCE),
        )),
    }
}

/// The working colour of a peniko sRGB colour.
pub fn working_color(color: peniko::Color) -> WorkingColor {
    WorkingColor::new(
        color
            .convert::<waterui_graphics::draw::LinearDisplayP3>()
            .components,
    )
}

/// The engine paint of a peniko gradient.
fn gradient_paint(gradient: &peniko::Gradient) -> Paint {
    let stops: Vec<ColorStop> = gradient
        .stops
        .iter()
        .map(|stop| ColorStop {
            offset: stop.offset,
            color: dynamic_working_color(stop.color),
        })
        .collect();
    let extend = extend(gradient.extend);
    let interpolation = interpolation(gradient.interpolation_cs);
    match gradient.kind {
        peniko::GradientKind::Linear(position) => Paint::Linear(LinearGradient {
            start: position.start,
            end: position.end,
            stops,
            extend,
            interpolation,
        }),
        peniko::GradientKind::Radial(position) => Paint::Radial(RadialGradient {
            start_center: position.start_center,
            start_radius: f64::from(position.start_radius),
            end_center: position.end_center,
            end_radius: f64::from(position.end_radius),
            stops,
            extend,
            interpolation,
        }),
        peniko::GradientKind::Sweep(position) => Paint::Sweep(SweepGradient {
            center: position.center,
            start_angle: f64::from(position.start_angle),
            end_angle: f64::from(position.end_angle),
            stops,
            extend,
            interpolation,
        }),
    }
}

/// The working colour of a run-time colour space colour.
fn dynamic_working_color(color: peniko::color::DynamicColor) -> WorkingColor {
    let linear = color.convert(peniko::color::ColorSpaceTag::LinearSrgb);
    let alpha = linear.to_alpha_color::<waterui_graphics::draw::LinearSrgb>();
    WorkingColor::new(
        alpha
            .convert::<waterui_graphics::draw::LinearDisplayP3>()
            .components,
    )
}

/// The gradient interpolation space the engine supports; any other space is
/// a programmer error on the waterui side, not something to approximate.
fn interpolation(cs: peniko::color::ColorSpaceTag) -> Interpolation {
    match cs {
        peniko::color::ColorSpaceTag::Srgb => Interpolation::SrgbEncoded,
        peniko::color::ColorSpaceTag::LinearSrgb => Interpolation::Working,
        other => panic!(
            "hydrolysis renderer: gradient interpolation space {other:?} has no engine mapping"
        ),
    }
}

const fn extend(extend: peniko::Extend) -> Extend {
    match extend {
        peniko::Extend::Pad => Extend::Pad,
        peniko::Extend::Repeat => Extend::Repeat,
        peniko::Extend::Reflect => Extend::Reflect,
    }
}

const fn sampling(quality: peniko::ImageQuality) -> Sampling {
    match quality {
        peniko::ImageQuality::Low => Sampling::Nearest,
        peniko::ImageQuality::Medium | peniko::ImageQuality::High => Sampling::Linear,
    }
}

/// The engine blend mode of a peniko `mix`/`compose` pair. The engine carries
/// one combined mode; peniko's separable pair collapses where Porter-Duff and
/// CSS compositing overlap, and the remaining combinations have no engine
/// equivalent.
fn blend_mode(mode: BlendMode) -> waterui_graphics::draw::BlendMode {
    use peniko::{Compose, Mix};
    match (mode.mix, mode.compose) {
        (Mix::Normal, Compose::SrcOver) => waterui_graphics::draw::BlendMode::Normal,
        (Mix::Multiply, Compose::SrcOver) => waterui_graphics::draw::BlendMode::Multiply,
        (Mix::Screen, Compose::SrcOver) => waterui_graphics::draw::BlendMode::Screen,
        (Mix::Overlay, Compose::SrcOver) => waterui_graphics::draw::BlendMode::Overlay,
        (Mix::Darken, Compose::SrcOver) => waterui_graphics::draw::BlendMode::Darken,
        (Mix::Lighten, Compose::SrcOver) => waterui_graphics::draw::BlendMode::Lighten,
        (Mix::ColorDodge, Compose::SrcOver) => waterui_graphics::draw::BlendMode::ColorDodge,
        (Mix::ColorBurn, Compose::SrcOver) => waterui_graphics::draw::BlendMode::ColorBurn,
        (Mix::HardLight, Compose::SrcOver) => waterui_graphics::draw::BlendMode::HardLight,
        (Mix::SoftLight, Compose::SrcOver) => waterui_graphics::draw::BlendMode::SoftLight,
        (Mix::Difference, Compose::SrcOver) => waterui_graphics::draw::BlendMode::Difference,
        (Mix::Exclusion, Compose::SrcOver) => waterui_graphics::draw::BlendMode::Exclusion,
        (Mix::Hue, Compose::SrcOver) => waterui_graphics::draw::BlendMode::Hue,
        (Mix::Saturation, Compose::SrcOver) => waterui_graphics::draw::BlendMode::Saturation,
        (Mix::Color, Compose::SrcOver) => waterui_graphics::draw::BlendMode::Color,
        (Mix::Luminosity, Compose::SrcOver) => waterui_graphics::draw::BlendMode::Luminosity,
        (Mix::Normal, Compose::Clear) => waterui_graphics::draw::BlendMode::Clear,
        (Mix::Normal, Compose::Copy) => waterui_graphics::draw::BlendMode::Src,
        (Mix::Normal, Compose::Dest) => waterui_graphics::draw::BlendMode::Dst,
        (Mix::Normal, Compose::DestOver) => waterui_graphics::draw::BlendMode::DestOver,
        (Mix::Normal, Compose::SrcIn) => waterui_graphics::draw::BlendMode::SrcIn,
        (Mix::Normal, Compose::DestIn) => waterui_graphics::draw::BlendMode::DestIn,
        (Mix::Normal, Compose::SrcOut) => waterui_graphics::draw::BlendMode::SrcOut,
        (Mix::Normal, Compose::DestOut) => waterui_graphics::draw::BlendMode::DestOut,
        (Mix::Normal, Compose::SrcAtop) => waterui_graphics::draw::BlendMode::SrcAtop,
        (Mix::Normal, Compose::DestAtop) => waterui_graphics::draw::BlendMode::DestAtop,
        (Mix::Normal, Compose::Xor) => waterui_graphics::draw::BlendMode::Xor,
        (Mix::Normal, Compose::PlusLighter) => waterui_graphics::draw::BlendMode::PlusLighter,
        (mix, compose) => panic!(
            "hydrolysis renderer: blend mode ({mix:?}, {compose:?}) has no engine equivalent"
        ),
    }
}

/// Maps a paint's own coordinates through `transform`. Solid colours carry no
/// space; image patterns carry their transform themselves; gradient geometry
/// is moved point by point, so a non-uniform scale on a radial gradient keeps
/// its centre and scales its radii by the transform's mean scale.
pub fn transform_paint(paint: Paint, transform: Option<Affine>) -> Paint {
    let Some(transform) = transform else {
        return paint;
    };
    if transform == Affine::IDENTITY {
        return paint;
    }
    let scale = {
        let [a, b, c, d, _, _] = transform.as_coeffs();
        f64::midpoint(a.hypot(b), c.hypot(d))
    };
    match paint {
        Paint::Linear(mut gradient) => {
            gradient.start = transform * gradient.start;
            gradient.end = transform * gradient.end;
            Paint::Linear(gradient)
        }
        Paint::Radial(mut gradient) => {
            gradient.start_center = transform * gradient.start_center;
            gradient.end_center = transform * gradient.end_center;
            gradient.start_radius *= scale;
            gradient.end_radius *= scale;
            Paint::Radial(gradient)
        }
        Paint::Sweep(mut gradient) => {
            gradient.center = transform * gradient.center;
            Paint::Sweep(gradient)
        }
        Paint::Mesh(mesh) => Paint::Mesh(
            waterui_graphics::draw::MeshGradient::new(
                mesh.columns(),
                mesh.rows(),
                mesh.points().iter().map(|p| transform * *p).collect(),
                mesh.colors().to_vec(),
            )
            .interpolation(mesh.interpolation_mode()),
        ),
        Paint::Image(mut pattern) => {
            pattern.transform = transform * pattern.transform;
            Paint::Image(pattern)
        }
        other @ (Paint::Solid(_) | Paint::Shader(_)) => other,
        Paint::Transformed(mut paint) => {
            paint.transform = transform * paint.transform;
            Paint::Transformed(paint)
        }
    }
}

/// Scales a paint's alpha where a legacy alpha channel rides on the brush.
fn alpha_scaled_paint(paint: Paint, alpha: f32) -> Paint {
    match paint {
        Paint::Solid(color) => Paint::Solid(color.with_alpha(color.components[3] * alpha)),
        _ => paint,
    }
}

/// The drawing facade widget chrome records through.
///
impl Recording {
    /// Records `body` into a picture drawn under `transform` — how detached
    /// chrome (context menus, caret overlays, navigation bars) splices a
    /// sealed subtree into the scene. A [`Recorder`] exists only inside a
    /// [`Content::record`] closure under the engine's target contract, so
    /// `body` runs there and the finished content freezes into the picture.
    /// Chrome does not bind to a layer's layout size, so the recording reads
    /// a fresh [`LayoutSize`].
    pub(crate) fn record_picture(&mut self, transform: Affine, body: impl FnOnce(&mut Recorder)) {
        let picture = Content::record(&LayoutSize::new(), body).into_picture();
        if !picture.display_list().is_empty() {
            self.draw_picture(transform, picture);
        }
    }
}

impl crate::renderer::chrome::LiveSource for Recording {
    fn record(
        &self,
        recorder: &mut Recorder,
        resources: &SceneResources,
    ) -> Option<LoweredResources> {
        Some(self.record_on(recorder, resources))
    }

    fn recording(&self) -> Option<&Recording> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use kurbo::Point;
    use waterui_graphics::draw::{MeshColorInterpolation, MeshGradient};

    use super::*;

    #[test]
    fn transform_paint_keeps_mesh_interpolation() {
        let mesh = MeshGradient::new(
            1,
            1,
            vec![
                Point::new(0.0, 0.0),
                Point::new(10.0, 0.0),
                Point::new(0.0, 10.0),
                Point::new(10.0, 10.0),
            ],
            vec![
                WorkingColor::BLACK,
                WorkingColor::WHITE,
                WorkingColor::WHITE,
                WorkingColor::BLACK,
            ],
        )
        .interpolation(MeshColorInterpolation::Smoothstep);
        let Paint::Mesh(transformed) =
            transform_paint(Paint::Mesh(mesh), Some(Affine::translate((4.0, 2.0))))
        else {
            panic!("a mesh paint stays a mesh paint");
        };
        assert_eq!(
            transformed.interpolation_mode(),
            MeshColorInterpolation::Smoothstep
        );
        assert_eq!(transformed.points()[0], Point::new(4.0, 2.0));
    }

    /// A host mounting Hydrolysis recordings on a `Null` engine's surface
    /// root, keeping the installed recording's resources the way the layer
    /// mount does: the set of the recording a frame replaces is let go only
    /// once its replacement is installed.
    struct NullMount {
        engine: Rc<cherenkov::Engine<cherenkov::testing::Null>>,
        probe: std::sync::mpsc::Receiver<cherenkov::testing::Event>,
        surface: cherenkov::Surface<cherenkov::testing::Null>,
        resources: SceneResources,
        installed: Option<LoweredResources>,
    }

    impl NullMount {
        fn new() -> Self {
            let (events, probe) = std::sync::mpsc::channel();
            let engine = Rc::new(
                cherenkov::Engine::<cherenkov::testing::Null>::new(
                    cherenkov::testing::NullConfig {
                        events,
                        reject: std::collections::HashSet::new(),
                        image_limits: waterui_graphics::draw::ImageLimits::UNLIMITED,
                    },
                )
                .expect("the null engine starts"),
            );
            let surface = engine
                .surface(
                    cherenkov::Offscreen::new((8, 8), cherenkov::OffscreenFormat::LinearF16),
                    || {},
                )
                .expect("surface");
            let resources =
                SceneResources::over(waterui_graphics::SceneResources::new(Rc::clone(&engine)));
            Self {
                engine,
                probe,
                surface,
                resources,
                installed: None,
            }
        }

        /// One host frame: lower `recording`, install it, let go of the
        /// set it replaces, render. Returns the frame's render-thread
        /// events.
        fn frame(&mut self, recording: &Recording) -> Vec<cherenkov::testing::Event> {
            let mut lowered = None;
            let content = self.surface.record(|recorder| {
                lowered = Some(recording.record_on(recorder, &self.resources));
            });
            self.surface.update(|tx| {
                tx[self.surface.root()].content(content);
            });
            drop(std::mem::replace(&mut self.installed, lowered));
            self.engine
                .render(cherenkov::FrameTime::now())
                .expect("render");
            self.probe.try_iter().collect()
        }
    }

    /// The HWUI text seam: a recording's `Text` op names the layout by
    /// lease, and the lowered resources keep it live until the installed
    /// content is replaced (water-rs/waterui#1811).
    #[test]
    fn a_platform_text_op_installs_its_layout_lease() {
        let ids = crate::hwui::text::TextLayoutIds::new();
        let wire = ids.acquire().expect("a fresh table has ids");
        let resource = Arc::new(crate::hwui::text::TextLayoutResource::new(wire, ids));
        let weak = Arc::downgrade(&resource);
        let layout_id = waterui_graphics::draw::TextLayoutId::new(u64::from(wire));

        let mut recording = Recording::new();
        recording.draw_platform_text(
            Arc::clone(&resource),
            Affine::translate((3.0, 4.0)),
            Some(Rect::new(0.0, 0.0, 30.0, 20.0)),
        );

        // The resource also survives a clone through `append` into a placed
        // recording — the same lease, under a composed transform.
        let mut placed = Recording::new();
        placed.append(&recording, Affine::translate((7.0, 9.0)));
        drop(recording);

        let mount = NullMount::new();
        let mut lowered = None;
        let content = mount.surface.record(|recorder| {
            lowered = Some(placed.record_on(recorder, &mount.resources));
        });
        assert!(content.references(waterui_graphics::draw::ResourceId::TextLayout(layout_id)));

        drop(placed);
        drop(resource);
        assert!(
            weak.upgrade().is_some(),
            "installed content keeps the layout lease live"
        );
        drop(lowered);
        assert!(
            weak.upgrade().is_none(),
            "the last installed set releases the lease"
        );
    }

    /// A recording of one text run in `font`, over a fill painted with
    /// `photo` when there is one.
    fn page(font: &FontData, photo: Option<&peniko::ImageData>) -> Recording {
        let mut recording = Recording::new();
        if let Some(photo) = photo {
            recording.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                &peniko::Brush::Image(peniko::ImageBrush::new(photo.clone())),
                None,
                &Rect::new(0.0, 0.0, 4.0, 4.0),
            );
        }
        recording.glyphs(&GlyphRun {
            font,
            font_size: 12.0,
            normalized_coords: &[],
            transform: Affine::IDENTITY,
            brush: &peniko::Brush::Solid(peniko::color::palette::css::BLACK),
            brush_alpha: 1.0,
            style: peniko::StyleRef::Fill(Fill::NonZero),
            glyphs: &[Glyph {
                id: 1,
                x: 0.0,
                y: 10.0,
            }],
        });
        recording
    }

    /// Every id of `kind` the events report, in order.
    fn ids<T>(
        events: &[cherenkov::testing::Event],
        kind: impl Fn(&cherenkov::testing::Event) -> Option<T>,
    ) -> Vec<T> {
        events.iter().filter_map(kind).collect()
    }

    /// The identity caches hold a font or image registration exactly as long
    /// as an installed recording names it (water-rs/waterui#2787): a photo
    /// the next frame stops drawing is unregistered in that frame, the font
    /// every frame draws stays registered and is never re-uploaded, and once
    /// nothing names either the caches are empty and the engine holds
    /// nothing. The `Null` backend panics on a render that draws a removed
    /// resource, so a registration released while still drawn fails here too.
    #[test]
    fn a_cached_registration_lives_exactly_as_long_as_a_recording_names_it() {
        use cherenkov::testing::Event;
        let added_fonts = |e: &Event| matches!(e, Event::AddFont(_)).then_some(());
        let removed_fonts = |e: &Event| matches!(e, Event::RemoveFont(_)).then_some(());
        let added_images = |e: &Event| match e {
            Event::AddImage(id) => Some(*id),
            _ => None,
        };
        let removed_images = |e: &Event| match e {
            Event::RemoveImage(id) => Some(*id),
            _ => None,
        };
        let photo = |fill: u8| peniko::ImageData {
            data: peniko::Blob::from(vec![fill; 4 * 4 * 4]),
            format: peniko::ImageFormat::Rgba8,
            alpha_type: peniko::ImageAlphaType::Alpha,
            width: 4,
            height: 4,
        };
        let font = FontData::new(peniko::Blob::from(vec![0u8; 64]), 0);
        let (first, second) = (photo(1), photo(2));
        let mut mount = NullMount::new();

        let events = mount.frame(&page(&font, Some(&first)));
        assert_eq!(ids(&events, added_fonts).len(), 1, "{events:?}");
        let first_id = ids(&events, added_images);
        assert_eq!(first_id.len(), 1, "{events:?}");
        assert_eq!(mount.resources.cached(), (1, 1));
        assert_eq!(mount.resources.take_registration_stats(), (1, 1));

        // The same resources in a fresh recording: found by identity, not
        // registered again, and not released while the replaced recording
        // that also names them is let go.
        let events = mount.frame(&page(&font, Some(&first)));
        assert!(ids(&events, added_images).is_empty(), "{events:?}");
        assert!(ids(&events, removed_images).is_empty(), "{events:?}");
        assert!(ids(&events, removed_fonts).is_empty(), "{events:?}");
        assert_eq!(mount.resources.take_registration_stats(), (0, 0));
        assert_eq!(mount.resources.cached(), (1, 1));

        // The next photo replaces the first: the first leaves the cache and
        // the engine in the frame that stops drawing it; the font stays.
        let events = mount.frame(&page(&font, Some(&second)));
        assert_eq!(ids(&events, added_images).len(), 1, "{events:?}");
        assert_eq!(ids(&events, removed_images), first_id, "{events:?}");
        assert!(ids(&events, removed_fonts).is_empty(), "{events:?}");
        assert_eq!(mount.resources.take_registration_stats(), (0, 1));
        assert_eq!(mount.resources.cached(), (1, 1));

        // Nothing named any more: nothing cached, nothing registered.
        let events = mount.frame(&Recording::new());
        assert_eq!(ids(&events, removed_images).len(), 1, "{events:?}");
        assert_eq!(ids(&events, removed_fonts).len(), 1, "{events:?}");
        assert_eq!(mount.resources.cached(), (0, 0));
    }

    /// An image the engine's limits admit alone, registered beside images
    /// that leave less of the image budget than it needs, reports the
    /// engine's `OverBudget` error instead of panicking the window or
    /// standing in a resampled image (water-rs/waterui#2755), and an op
    /// painting with it is not drawn.
    #[test]
    fn an_image_past_the_image_budget_reports_the_error() {
        let (events, _events) = std::sync::mpsc::channel();
        let limits = waterui_graphics::draw::ImageLimits {
            max_dimension: 8,
            max_bytes: 64 * 4,
            ..waterui_graphics::draw::ImageLimits::UNLIMITED
        };
        let engine = std::rc::Rc::new(
            cherenkov::Engine::<cherenkov::testing::Null>::new(cherenkov::testing::NullConfig {
                events,
                reject: std::collections::HashSet::new(),
                image_limits: limits,
            })
            .expect("engine"),
        );
        let resources = SceneResources::over(waterui_graphics::SceneResources::new(engine));
        let mut names = resources.recording();
        let square = |side: u32, fill: u8| peniko::ImageData {
            data: peniko::Blob::from(vec![fill; 4 * side as usize * side as usize]),
            format: peniko::ImageFormat::Rgba8,
            alpha_type: peniko::ImageAlphaType::Alpha,
            width: side,
            height: side,
        };

        // 144 bytes, then 144 more beside them: 112 are left.
        resources
            .image(&mut names, &square(6, 1))
            .expect("the first image fits");
        let rejected = square(6, 2);
        match resources.image(&mut names, &rejected) {
            Err(ResourceError::OverBudget {
                width,
                height,
                available,
                limits: reported,
            }) => assert_eq!((width, height, available, reported), (6, 6, 112, limits)),
            other => panic!("an image past the budget was registered: {other:?}"),
        }

        // A brush painting with it reports the same error, and the op that
        // paints with it is not drawn.
        let brush = peniko::Brush::Image(peniko::ImageBrush::new(rejected));
        let paint = resources.paint(&mut names, &brush);
        assert!(
            matches!(paint, Err(ResourceError::OverBudget { .. })),
            "{paint:?}"
        );
        assert!(drawable(paint).is_none());

        // The refused image is not cached: only the first is listed.
        assert_eq!(resources.cached(), (0, 1));
    }

    /// A decoded source the engine's limits admit keeps its own pixels;
    /// a larger one is resampled to the fit before it reaches the
    /// registration — the oversized-decode case from water-rs/waterui#1806.
    #[test]
    fn an_oversized_image_is_resampled_to_the_engine_limits() {
        let source = peniko::ImageData {
            data: peniko::Blob::from(vec![255u8; 16 * 4 * 4]),
            format: peniko::ImageFormat::Rgba8,
            alpha_type: peniko::ImageAlphaType::Alpha,
            width: 16,
            height: 4,
        };
        let limits = waterui_graphics::draw::ImageLimits {
            max_dimension: 8,
            max_bytes: 64 * 4,
            ..waterui_graphics::draw::ImageLimits::UNLIMITED
        };
        let (width, height, bytes) = fit_image_to_limits(&source, limits);
        assert_eq!((width, height), (8, 2));
        assert_eq!(bytes.len(), 8 * 2 * 4);
        let (width, height, bytes) =
            fit_image_to_limits(&source, waterui_graphics::draw::ImageLimits::UNLIMITED);
        assert_eq!((width, height), (16, 4));
        assert_eq!(&*bytes, &[255u8; 16 * 4 * 4][..]);
    }
}
