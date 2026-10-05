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
//! lower time through [`SceneResources`], which keeps the handles alive and
//! deduplicates registrations across frames.

use core::fmt;
use kurbo::{Affine, Rect, Shape, Vec2};
use peniko::{BlendMode, Fill, FontData};
use rustc_hash::FxHashMap;
use std::sync::Arc;

use waterui_graphics::draw::{
    BlendSpace, ColorStop, Content, Draw, EvenOdd, Extend, Glyph as EngineGlyph, GlyphStyle, Group,
    ImageId, ImagePattern, Interpolation, LayoutSize, LinearGradient, Paint, RadialGradient,
    Recorder, Sampling, Shadow, ShapeData, SweepGradient, WorkingColor,
};
use waterui_graphics::{
    FontSource, HeldResources, ImageColorSpace, ImageData, RecordingResources, Rgba8,
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
    Fill {
        rule: Fill,
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
        rule: Fill,
        blend: BlendMode,
        opacity: f32,
        transform: Affine,
        clip: ShapeData,
    },
    Picture {
        transform: Affine,
        picture: waterui_graphics::draw::Picture,
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
/// device's texture limit or its per-image budget — is Lanczos3-resampled
/// to the largest admitted size at the same aspect ratio, so the
/// registration below still holds the engine's contract rather than
/// panicking on it. Returns `(width, height, bytes)`.
fn fit_image_to_limits(
    image: &peniko::ImageData,
    limits: waterui_graphics::draw::ImageLimits,
) -> (u32, u32, Arc<[u8]>) {
    let (width, height) = limits.fit(image.width, image.height);
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
/// host built over its `Engine`; the handles live here for as long as a frame
/// can show the recording that names them. The table alone already dedups by
/// content hash — the identity key here keeps that hash, and the engine
/// upload behind a cache miss, out of the per-frame lowering path.
pub struct SceneResources {
    /// The `SceneContent::build_scene` resource table, shared with every scene
    /// view this engine draws.
    inner: waterui_graphics::SceneResources,
    fonts: std::cell::RefCell<
        FxHashMap<FontKey, waterui_graphics::Registered<waterui_graphics::draw::FontId>>,
    >,
    images: std::cell::RefCell<
        FxHashMap<ImageKey, waterui_graphics::Registered<waterui_graphics::draw::ImageId>>,
    >,
    /// Fresh engine registrations since the last
    /// [`Self::take_registration_stats`] — the frame-work counters' evidence
    /// that fonts and images are not re-registered per frame.
    font_registrations: std::cell::Cell<u64>,
    image_registrations: std::cell::Cell<u64>,
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
            .field("fonts", &self.fonts.borrow().len())
            .field("images", &self.images.borrow().len())
            .finish_non_exhaustive()
    }
}

impl SceneResources {
    /// Creates caches served by `engine`.
    pub(crate) fn new(engine: &std::rc::Rc<crate::engine::GpuEngine>) -> Self {
        Self {
            inner: waterui_graphics::SceneResources::with_shaders(engine.clone(), engine.clone()),
            fonts: std::cell::RefCell::new(FxHashMap::default()),
            images: std::cell::RefCell::new(FxHashMap::default()),
            font_registrations: std::cell::Cell::new(0),
            image_registrations: std::cell::Cell::new(0),
        }
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
    pub(crate) fn font(
        &self,
        names: &mut RecordingResources<'_>,
        font: &FontData,
    ) -> waterui_graphics::draw::FontId {
        let key = FontKey {
            blob: font.data.id(),
            index: font.index,
        };
        if let Some(registered) = self.fonts.borrow().get(&key) {
            return names.name(registered);
        }
        let source = FontSource::bytes(Arc::<[u8]>::from(font.data.data())).with_index(font.index);
        let registered = self
            .inner
            .font(source)
            .expect("hydrolysis renderer: engine rejected a font source it must accept");
        let id = names.name(&registered);
        self.fonts.borrow_mut().insert(key, registered);
        self.font_registrations
            .set(self.font_registrations.get() + 1);
        id
    }

    /// The registered image id for `image`, uploading the pixels on first use
    /// and holding the registration for `names`' recording.
    pub(crate) fn image(
        &self,
        names: &mut RecordingResources<'_>,
        image: &peniko::ImageData,
    ) -> ImageId {
        let key = ImageKey {
            blob: image.data.id(),
            width: image.width,
            height: image.height,
            format: image.format as u8,
            alpha: image.alpha_type as u8,
        };
        if let Some(registered) = self.images.borrow().get(&key) {
            return names.name(registered);
        }
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
        let registered = self
            .inner
            .image(data)
            .expect("hydrolysis renderer: engine rejected an image it must accept");
        let id = names.name(&registered);
        self.images.borrow_mut().insert(key, registered);
        self.image_registrations
            .set(self.image_registrations.get() + 1);
        id
    }

    /// The engine paint for `brush`, registering any image it names and
    /// holding it for `names`' recording.
    pub(crate) fn paint(&self, names: &mut RecordingResources<'_>, brush: &peniko::Brush) -> Paint {
        match brush {
            peniko::Brush::Solid(color) => Paint::Solid(working_color(*color)),
            peniko::Brush::Gradient(gradient) => gradient_paint(gradient),
            peniko::Brush::Image(brush) => {
                let pattern = ImagePattern {
                    image: self.image(names, &brush.image),
                    transform: Affine::IDENTITY,
                    extend_x: extend(brush.sampler.x_extend),
                    extend_y: extend(brush.sampler.y_extend),
                    sampling: sampling(brush.sampler.quality),
                };
                Paint::Image(pattern)
            }
        }
    }

    /// `brush` mapped through its own coordinate transform.
    pub(crate) fn transformed_paint(
        &self,
        names: &mut RecordingResources<'_>,
        brush: &peniko::Brush,
        brush_transform: Option<Affine>,
    ) -> Paint {
        transform_paint(self.paint(names, brush), brush_transform)
    }
}

impl Recording {
    /// Creates an empty recording.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Clears all recorded commands.
    pub(crate) fn reset(&mut self) {
        self.ops.clear();
        self.open_layers = 0;
    }

    /// Whether the recording encodes any visible content.
    ///
    /// Layer scopes alone emit clip geometry in the lowered content, matching
    /// what the old encoder counted as non-empty.
    pub(crate) const fn is_empty(&self) -> bool {
        self.ops.is_empty()
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
            rule,
            transform,
            brush: brush.clone(),
            brush_transform,
            shape: shape_data(rule, shape),
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
            rule,
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
        self.ops
            .extend(other.ops.iter().cloned().map(|op| match op {
                Op::Fill {
                    rule,
                    transform,
                    brush,
                    brush_transform,
                    shape,
                } => Op::Fill {
                    rule,
                    transform: placement * transform,
                    brush,
                    brush_transform,
                    shape,
                },
                Op::FillPaint {
                    transform,
                    paint,
                    shape,
                } => Op::FillPaint {
                    transform: placement * transform,
                    paint,
                    shape,
                },
                Op::StrokePaint {
                    stroke,
                    transform,
                    paint,
                    shape,
                } => Op::StrokePaint {
                    stroke,
                    transform: placement * transform,
                    paint,
                    shape,
                },
                Op::Glyphs {
                    font,
                    font_size,
                    coords,
                    transform,
                    brush,
                    brush_alpha,
                    style,
                    glyphs,
                } => Op::Glyphs {
                    font,
                    font_size,
                    coords,
                    transform: placement * transform,
                    brush,
                    brush_alpha,
                    style,
                    glyphs,
                },
                Op::BlurredRoundedRect {
                    transform,
                    rect,
                    color,
                    radius,
                    sigma,
                } => Op::BlurredRoundedRect {
                    transform: placement * transform,
                    rect,
                    color,
                    radius,
                    sigma,
                },
                Op::Shadow {
                    transform,
                    shape,
                    color,
                    sigma,
                } => Op::Shadow {
                    transform: placement * transform,
                    shape,
                    color,
                    sigma,
                },
                Op::PushGroup {
                    rule,
                    blend,
                    opacity,
                    transform,
                    clip,
                } => Op::PushGroup {
                    rule,
                    blend,
                    opacity,
                    transform: placement * transform,
                    clip,
                },
                Op::Picture { transform, picture } => Op::Picture {
                    transform: placement * transform,
                    picture,
                },
                Op::PopLayer => Op::PopLayer,
            }));
    }

    /// Push scopes still open, for the tracked-stack invariant the flush
    /// asserts.
    pub(crate) const fn open_clip_count(&self) -> u32 {
        self.open_layers
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

    /// Each fill op's shape bounds with the transform it was drawn under,
    /// in op order — `transform * shape` is the rect the fill paints. For
    /// tests asserting paint geometry.
    #[cfg(test)]
    pub(crate) fn fill_bounds(&self) -> impl Iterator<Item = (Affine, Rect)> {
        self.ops.iter().filter_map(|op| match op {
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
    /// The caller keeps the [`HeldResources`] for as long as the content is
    /// installed on a layer — the mounts own that lifetime, releasing a
    /// set only once the replacement content is installed.
    ///
    /// # Panics
    /// Panics when a layer push has no matching pop — the flush keeps the
    /// invariant on every code path it accepts.
    pub(crate) fn record_on(
        &self,
        recorder: &mut Recorder,
        resources: &SceneResources,
    ) -> HeldResources {
        let mut names = resources.inner.recording();
        let mut index = 0;
        lower(&self.ops, &mut index, recorder, resources, &mut names);
        assert_eq!(
            index,
            self.ops.len(),
            "hydrolysis recording: layer pop without a matching push"
        );
        names.finish()
    }
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
    names: &mut RecordingResources<'_>,
) {
    while *index < ops.len() {
        let op = &ops[*index];
        *index += 1;
        match op {
            Op::Fill {
                transform,
                brush,
                brush_transform,
                shape,
                ..
            } => {
                let paint = resources.transformed_paint(names, brush, *brush_transform);
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
                let mut paint = resources.paint(names, brush);
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
            max_texels: 64,
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
