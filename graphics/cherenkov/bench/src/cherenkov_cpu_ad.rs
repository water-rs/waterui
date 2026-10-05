//! `cherenkov-cpu` adapter: the CPU banded exact-area rasterizer.
//!
//! Route: the front-end records a display list per content layer, lowered on
//! the render thread into flattened device-space edge lists drawn by a
//! rayon-banded signed-area coverage rasterizer into an f32 framebuffer
//! (premultiplied linear Display P3 — the suite's working space end to end).
//! Readback is f16-rounded by default; `CHERENKOV_CPU_READBACK=f32` keeps the
//! raw f32s. There is no GPU timestamp source; `gpu_seconds` is always `null`.

use std::collections::{BTreeSet, HashMap};

use cherenkov::{
    Draw as _, Engine as CpuEngine, Fixed, Layer as CpuLayer, LayerEdit, Offscreen,
    OffscreenFormat, RenderError, ResourceError, Surface, Transaction,
};
use cherenkov_cpu::{Raster, RasterConfig};
use cherenkov_oracle::present::presented_srgb_to_working;
use cherenkov_scene::{
    BackdropEffectSpec, BackdropFilter, BlendMode, BlendSpace, ColorSpace, Draw as SceneDraw,
    Feature, FilterBlend, GroupItem, ImageColorSpace, ImageEncoding, Item, Layer as SceneLayer,
    LayerFilter, Motion, ResourceHash,
};
use filtrate::{FilterExt, FilterImage, filters};
use kurbo::{Affine, BezPath, Circle, Ellipse, Line, Rect, RoundedRect, Vec2};

use crate::convert::{
    self, Blobs, Op, ShapeKind, engine_blend, group_op, op, shape_kind, text_op, working,
};
use crate::memory::{AdapterMemory, EngineBytes, Reading};
use crate::motion::{Clock, LayerMotion, LayerProjection, motion_animation};
use crate::timing::Timings;
use crate::{
    BenchError, Counters, DeviceInfo, EncodeInput, Engine, EngineInfo, GpuSample, PresentKind,
    Submit,
};

/// A scene shape as a live operand: the [`ShapeKind`] kinds plus the
/// fill rule carried on the path variant, so the slot value is
/// self-contained.
#[derive(Clone, PartialEq)]
enum LiveShape {
    /// `ShapeKind::Rect`.
    Rect(Rect),
    /// `ShapeKind::RoundedRect`.
    RoundedRect(RoundedRect),
    /// `ShapeKind::Continuous`.
    Continuous(cherenkov::ContinuousRect),
    /// `ShapeKind::Circle`.
    Circle(Circle),
    /// `ShapeKind::Ellipse`.
    Ellipse(Ellipse),
    /// `ShapeKind::Line`.
    Line(Line),
    /// A path with the fill rule it was recorded under.
    Path {
        /// The path.
        path: BezPath,
        /// The fill rule.
        rule: cherenkov::FillRule,
    },
}

impl LiveShape {
    /// The live operand for `shape` (a path records `rule`).
    fn of(shape: &ShapeKind, rule: cherenkov::FillRule) -> Self {
        match shape {
            ShapeKind::Rect(s) => Self::Rect(*s),
            ShapeKind::RoundedRect(s) => Self::RoundedRect(*s),
            ShapeKind::Continuous(s) => Self::Continuous(*s),
            ShapeKind::Circle(s) => Self::Circle(*s),
            ShapeKind::Ellipse(s) => Self::Ellipse(*s),
            ShapeKind::Line(s) => Self::Line(*s),
            ShapeKind::Path { path, .. } => Self::Path {
                path: path.clone(),
                rule,
            },
        }
    }
}

impl cherenkov::Shape for LiveShape {
    fn semantic(&self) -> cherenkov::Semantic<'_> {
        match self {
            Self::Rect(s) => cherenkov::Semantic::Rect(*s),
            Self::RoundedRect(s) => cherenkov::Semantic::RoundedRect(*s),
            Self::Continuous(s) => cherenkov::Semantic::Continuous(*s),
            Self::Circle(s) => cherenkov::Semantic::Circle(*s),
            Self::Ellipse(s) => cherenkov::Semantic::Ellipse(*s),
            Self::Line(s) => cherenkov::Semantic::Line(*s),
            Self::Path { path, rule } => cherenkov::Semantic::Path(cherenkov::PathRef {
                elements: std::borrow::Cow::Borrowed(path.elements()),
                rule: *rule,
            }),
        }
    }
}

/// `op`'s shape operand, or `None` when it has none.
fn shape_op(op: &Op) -> Option<LiveShape> {
    match op {
        Op::Fill { shape, rule, .. } => Some(LiveShape::of(shape, front_rule(*rule))),
        Op::Stroke { shape, .. } | Op::Shadow { shape, .. } => {
            Some(LiveShape::of(shape, cherenkov::FillRule::NonZero))
        }
        Op::Glyphs { .. } | Op::Image { .. } | Op::Group { .. } | Op::Text { .. } => None,
    }
}

/// `op`'s paint operand.
fn paint_op(op: &Op) -> Option<cherenkov::Paint> {
    match op {
        Op::Fill { paint, .. } | Op::Stroke { paint, .. } | Op::Glyphs { paint, .. } => {
            Some(paint.clone())
        }
        Op::Shadow { .. } | Op::Image { .. } | Op::Group { .. } | Op::Text { .. } => None,
    }
}

/// `op`'s stroke-style operand.
fn stroke_op(op: &Op) -> Option<kurbo::Stroke> {
    match op {
        Op::Stroke { stroke, .. } => Some(stroke.clone()),
        _ => None,
    }
}

/// `op`'s shadow operand.
const fn shadow_op(op: &Op) -> Option<cherenkov::Shadow> {
    match op {
        Op::Shadow { shadow, .. } => Some(*shadow),
        _ => None,
    }
}

/// `op`'s glyph-run operand.
fn run_op(op: &Op) -> Option<cherenkov::GlyphRun> {
    match op {
        Op::Glyphs { run, .. } => Some(run.clone()),
        _ => None,
    }
}

/// `op`'s destination-rect operand.
const fn dst_op(op: &Op) -> Option<Rect> {
    match op {
        Op::Image { dst, .. } => Some(*dst),
        _ => None,
    }
}

/// The slot bindings a live op is recorded with: one per operand that
/// differs between frames.
#[derive(Default)]
struct LiveBindings {
    /// Image destination rectangle.
    dst: Option<nami::Binding<Rect>>,
    /// Fill/stroke/shadow shape.
    shape: Option<nami::Binding<LiveShape>>,
    /// Fill/stroke/glyph paint.
    paint: Option<nami::Binding<cherenkov::Paint>>,
    /// Stroke style.
    stroke: Option<nami::Binding<kurbo::Stroke>>,
    /// Shadow spec.
    shadow: Option<nami::Binding<cherenkov::Shadow>>,
    /// Glyph run.
    run: Option<nami::Binding<cherenkov::GlyphRun>>,
}

impl LiveBindings {
    /// Binds the operands that differ across `frames`.
    fn for_frames(frames: &[Op]) -> Self {
        let mut b = Self::default();
        let Some(base) = frames.first() else {
            return b;
        };
        if frames.iter().any(|f| dst_op(f) != dst_op(base)) {
            b.dst = dst_op(base).map(nami::binding);
        }
        if frames.iter().any(|f| shape_op(f) != shape_op(base)) {
            b.shape = shape_op(base).map(nami::binding);
        }
        if frames.iter().any(|f| paint_op(f) != paint_op(base)) {
            b.paint = paint_op(base).map(nami::binding);
        }
        if frames.iter().any(|f| stroke_op(f) != stroke_op(base)) {
            b.stroke = stroke_op(base).map(nami::binding);
        }
        if frames.iter().any(|f| shadow_op(f) != shadow_op(base)) {
            b.shadow = shadow_op(base).map(nami::binding);
        }
        if frames.iter().any(|f| run_op(f) != run_op(base)) {
            b.run = run_op(base).map(nami::binding);
        }
        b
    }

    /// Sets each bound operand to `op`'s value where it differs from `prev`.
    fn set(&self, op: &Op, prev: Option<&Op>) {
        if let Some(b) = &self.dst
            && prev.is_none_or(|p| dst_op(p) != dst_op(op))
        {
            b.set(dst_op(op).expect("bound image destination"));
        }
        if let Some(b) = &self.shape
            && prev.is_none_or(|p| shape_op(p) != shape_op(op))
        {
            b.set(shape_op(op).expect("bound op has a shape"));
        }
        if let Some(b) = &self.paint
            && prev.is_none_or(|p| paint_op(p) != paint_op(op))
        {
            b.set(paint_op(op).expect("bound op has a paint"));
        }
        if let Some(b) = &self.stroke
            && prev.is_none_or(|p| stroke_op(p) != stroke_op(op))
        {
            b.set(stroke_op(op).expect("bound op has a stroke"));
        }
        if let Some(b) = &self.shadow
            && prev.is_none_or(|p| shadow_op(p) != shadow_op(op))
        {
            b.set(shadow_op(op).expect("bound op has a shadow"));
        }
        if let Some(b) = &self.run
            && prev.is_none_or(|p| run_op(p) != run_op(op))
        {
            b.set(run_op(op).expect("bound op has a run"));
        }
    }
}

/// One live draw item: the op per frame and the bindings it is driven by.
struct LiveRun {
    /// Op index inside the owning content run.
    index: usize,
    /// The op per frame (`frames[n % len]`).
    frames: Vec<Op>,
    /// The bindings the varying operands were recorded with.
    bindings: LiveBindings,
    /// The frame index last set; `None` until the first advance.
    previous: Option<usize>,
}

impl LiveRun {
    /// Sets the bindings to frame `n`'s values where they differ.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "encode frame counts stay far below usize"
    )]
    fn advance(&mut self, frame: u64) {
        let n = frame as usize % self.frames.len();
        if self.previous == Some(n) {
            return;
        }
        let prev = self.previous.map(|p| &self.frames[p]);
        self.bindings.set(&self.frames[n], prev);
        self.previous = Some(n);
    }
}

/// One `Motion::Paint` target inside a content run: the draw's paint
/// operand records against `binding`, and setting it to `target` once the
/// content is installed animates the operand `from` → `target`.
struct PaintMotion {
    /// Op index inside the owning content run.
    index: usize,
    /// The paint operand's binding, at `from` until the first `set`.
    binding: nami::Binding<cherenkov::Paint>,
    /// The item's static paint — where the operand settles.
    target: cherenkov::Paint,
    /// How `from` moves to `target`.
    animation: cherenkov::Animation,
}

/// The operand a live op records: the binding when the operand varies
/// across frames, a constant otherwise.
fn live_or_const<T: Clone + 'static>(
    binding: Option<&nami::Binding<T>>,
    value: &T,
) -> cherenkov::Live<T> {
    binding.map_or_else(
        || nami::constant(value.clone()).into(),
        |b| b.clone().into(),
    )
}

/// Records `op` like [`record_op`], but with slot bindings for the
/// operands that vary across its frames.
#[expect(
    clippy::inline_always,
    reason = "the record closure must keep dev's per-op call-free codegen"
)]
#[inline(always)]
fn record_live(c: &mut cherenkov::Recorder, op: &Op, bindings: &LiveBindings) {
    match op {
        Op::Image {
            image,
            dst,
            sampling,
        } => c.image(*image, live_or_const(bindings.dst.as_ref(), dst), *sampling),
        Op::Fill { shape, rule, paint } => c.fill(
            live_or_const(
                bindings.shape.as_ref(),
                &LiveShape::of(shape, front_rule(*rule)),
            ),
            live_or_const(bindings.paint.as_ref(), paint),
        ),
        Op::Stroke {
            shape,
            stroke,
            paint,
        } => c.stroke(
            live_or_const(
                bindings.shape.as_ref(),
                &LiveShape::of(shape, cherenkov::FillRule::NonZero),
            ),
            live_or_const(bindings.stroke.as_ref(), stroke),
            live_or_const(bindings.paint.as_ref(), paint),
        ),
        Op::Shadow { shape, shadow } => c.shadow(
            live_or_const(
                bindings.shape.as_ref(),
                &LiveShape::of(shape, cherenkov::FillRule::NonZero),
            ),
            live_or_const(bindings.shadow.as_ref(), shadow),
        ),
        Op::Glyphs { run, paint } => c.glyphs(
            live_or_const(bindings.run.as_ref(), run),
            live_or_const(bindings.paint.as_ref(), paint),
        ),
        // Groups never target a live entry; members record plainly.
        Op::Group { group, ops } => c.group(Fixed(*group), |c| {
            for op in ops {
                record_op(c, op);
            }
        }),
        Op::Text { .. } => unreachable!("a text layer carries no live items"),
    }
}

/// Records `op` like [`record_op`], but with `motion`'s animated binding
/// for its paint operand. The op is always a paint-carrying draw —
/// `prep` rejects a `Motion::Paint` on a paint-less draw.
#[inline(always)]
fn record_motion(c: &mut cherenkov::Recorder, op: &Op, motion: &PaintMotion) {
    use nami::SignalExt as _;
    let paint: cherenkov::Live<cherenkov::Paint> =
        motion.binding.clone().with(motion.animation).into();
    match op {
        Op::Fill { shape, rule, .. } => {
            c.fill(
                nami::constant(LiveShape::of(shape, front_rule(*rule))),
                paint,
            );
        }
        Op::Stroke { shape, stroke, .. } => {
            c.stroke(
                nami::constant(LiveShape::of(shape, cherenkov::FillRule::NonZero)),
                nami::constant(stroke.clone()),
                paint,
            );
        }
        Op::Glyphs { run, .. } => c.glyphs(nami::constant(run.clone()), paint),
        Op::Image { .. } | Op::Shadow { .. } | Op::Group { .. } | Op::Text { .. } => {
            unreachable!("a paint motion only ever binds a paint operand")
        }
    }
}

/// Records `ops` with `live` slot bindings — identical to dev's record
/// loop, reached by scenes that carry no motions.
#[expect(
    clippy::inline_always,
    reason = "the record closure must keep dev's per-op call-free codegen"
)]
#[inline(always)]
fn record_ops_static(c: &mut cherenkov::Recorder, ops: &[Op], live: &[LiveRun]) {
    for (index, op) in ops.iter().enumerate() {
        match live.iter().find(|live| live.index == index) {
            Some(live) => record_live(c, op, &live.bindings),
            None => record_op(c, op),
        }
    }
}

/// Records `ops` with `live` slot bindings and `motions` animated paint
/// bindings, reached only on scenes that carry motions.
#[expect(
    clippy::inline_always,
    reason = "the record closure must keep dev's per-op call-free codegen"
)]
#[inline(always)]
fn record_ops(c: &mut cherenkov::Recorder, ops: &[Op], live: &[LiveRun], motions: &[PaintMotion]) {
    for (index, op) in ops.iter().enumerate() {
        match live.iter().find(|live| live.index == index) {
            Some(live) => record_live(c, op, &live.bindings),
            None => match motions.iter().find(|motion| motion.index == index) {
                Some(motion) => record_motion(c, op, motion),
                None => record_op(c, op),
            },
        }
    }
}

/// A maximal run of draw items, drawn as one layer's content.
struct ContentRun {
    /// The recorded ops.
    ops: Vec<Op>,
    /// Live items inside the run.
    live: Vec<LiveRun>,
    /// Paint motions inside the run.
    motions: Vec<PaintMotion>,
}

/// A prepared child item: a draw-item run wrapped in its own layer, or a
/// real child layer.
enum PrepItem {
    /// A maximal run of draw items, drawn as one layer's content.
    Content(ContentRun),
    /// A child scene layer.
    Layer(Box<PrepLayer>),
}

/// A scene layer lowered in `prepare`.
struct PrepLayer {
    /// Local transform.
    transform: Affine,
    /// Clip in the layer's own space.
    clip: Option<ShapeKind>,
    /// Group opacity.
    opacity: f64,
    /// Blend onto the parent.
    blend: cherenkov::BlendMode,
    /// Registered layer filter.
    filter: Option<cherenkov::Filter>,
    /// Scroll offset applied to content and children.
    scroll_offset: Vec2,
    /// The layer's own content — only when every draw precedes every child.
    own: ContentRun,
    /// Ordered children.
    items: Vec<PrepItem>,
    /// The layer's projective pose.
    projection: Option<LayerProjection>,
    /// The layer's one-time motion.
    motion: Option<LayerMotion>,
    /// The backdrop group this layer samples, if any.
    backdrop: Option<u32>,
    /// The member's per-member backdrop effect, if any.
    backdrop_effect: Option<BackdropEffectSpec>,
}

/// An engine layer plus the ops it records each frame.
struct ContentLayer {
    /// The layer handle; `None` for the surface root.
    layer: Option<CpuLayer>,
    /// Its recorded ops.
    ops: Vec<Op>,
    /// Live items inside `ops`.
    live: Vec<LiveRun>,
    /// Paint motions inside `ops`, started once the content installs.
    motions: Vec<PaintMotion>,
    /// The layer's one-time motion, committed on the first encode.
    motion: Option<LayerMotion>,
}

impl ContentLayer {
    /// The engine layer handle, resolving `None` to the surface root.
    fn handle<'a>(&'a self, surface: &'a Surface<Raster>) -> &'a CpuLayer {
        self.layer.as_ref().unwrap_or_else(|| surface.root())
    }
}

/// `cherenkov-cpu` adapter.
pub struct Cherenkov {
    info: EngineInfo,
    engine: CpuEngine<Raster>,
    surface: Option<Surface<Raster>>,
    /// Registered fonts per `(blob hash, face index)`.
    fonts: HashMap<(ResourceHash, u32), cherenkov::Font>,
    images: HashMap<(ResourceHash, ImageEncoding), cherenkov::ImageId>,
    image_handles: Vec<ImageHandle>,
    /// Registered filters referenced by prepared layers.
    filter_handles: Vec<cherenkov::Filter>,
    /// The backdrop groups created in `prepare`, alive while the surface
    /// is.
    backdrop_groups: HashMap<u32, cherenkov::BackdropGroup>,
    /// Layers holding recorded content, in draw order.
    content_layers: Vec<ContentLayer>,
    /// Whether any layer carries a `motion`.
    has_motion: bool,
    /// Whether the motion commits have been sent (first encode).
    motion_committed: bool,
    /// Whether any content layer carries live items.
    has_live: bool,
    /// Encode frames since `prepare` (`frames[n % len]` for live items).
    frame: u64,
    /// The fixed frame clock `submit` renders at.
    clock: Clock,
    /// Attributes each resolved GPU timing to the bench frame that
    /// rendered it.
    timings: Timings,
    counters: Counters,
    /// `--present` mode: the presentation kind `submit` applies to the
    /// readback before handing pixels over.
    present: Option<PresentKind>,
    /// The scene's declared display headroom for `submit`'s presentation.
    headroom: f32,
}

/// The features this slice executes faithfully.
fn cherenkov_features() -> Vec<Feature> {
    vec![
        Feature::Fill,
        Feature::Stroke,
        Feature::Path,
        Feature::EvenOdd,
        Feature::StrokeDash,
        Feature::ContinuousCorners,
        Feature::PaintTransform,
        Feature::LinearGradient,
        Feature::RadialGradient,
        Feature::SweepGradient,
        Feature::MeshGradient,
        Feature::GlyphStroke,
        Feature::GlyphTransform,
        Feature::Image,
        Feature::ImagePaint,
        // Display P3 PNGs upload verbatim; the `Rgba16F` path carries the
        // linear primaries as well.
        Feature::ImageColorSpace(ImageColorSpace::DisplayP3),
        Feature::ImageColorSpace(ImageColorSpace::LinearP3),
        Feature::ImageColorSpace(ImageColorSpace::LinearSrgb),
        Feature::ImageF16,
        Feature::ExtendNone,
        Feature::Clip,
        Feature::Opacity,
        Feature::Filter,
        Feature::Shadow,
        Feature::Glyphs,
        Feature::FontVariations,
        Feature::Scroll,
        Feature::Animation,
        Feature::HdrColor,
        Feature::WideGamut,
        Feature::Backdrop,
        Feature::BackdropBlur,
        Feature::BackdropColorMatrix,
        Feature::BackdropEffect,
        Feature::BackdropScale,
        Feature::Projective,
        // `sRGB` maps to `SrgbEncoded`; `linear-p3` and `linear-srgb` are
        // both linear interpolation, which is the working space already.
        Feature::InterpolationSpace(ColorSpace::Srgb),
        Feature::InterpolationSpace(ColorSpace::LinearP3),
        Feature::InterpolationSpace(ColorSpace::LinearSrgb),
        Feature::BlendSpace(BlendSpace::Linear),
        Feature::BlendSpace(BlendSpace::SrgbEncoded),
    ]
    .into_iter()
    .chain(BlendMode::ALL.into_iter().map(Feature::Blend))
    .collect()
}

/// The scene [`Feature`] a render-time unsupported name maps back to.
fn unsupported_feature(u: &str) -> Feature {
    match u {
        "sweep-gradient" => Feature::SweepGradient,
        "mesh-gradient" | "image" | "shader-paint" => Feature::Image,
        "blend-mode" => Feature::Blend(BlendMode::Normal),
        "blend-space" => Feature::BlendSpace(BlendSpace::SrgbEncoded),
        "projective-unclipped"
        | "projective-backdrop-member"
        | "projective-backdrop-cross-space" => Feature::Projective,
        "backdrop-unclipped"
        | "backdrop-footprint"
        | "backdrop-effect-sdf-path"
        | "backdrop-shader" => Feature::Backdrop,
        "filter" => Feature::Filter,
        "glyph-stroke" | "color-font" => Feature::Glyphs,
        "glyph-transform" => Feature::GlyphTransform,
        "shadow" => Feature::Shadow,
        _ => Feature::Fill,
    }
}

/// Maps a render-time error into a `BenchError`, keeping `Unsupported`
/// scenes reported rather than fatal.
fn render_error(e: RenderError) -> BenchError {
    match e {
        RenderError::Unsupported(u) => BenchError::Unsupported {
            engine: Cherenkov::NAME,
            feature: unsupported_feature(u),
            api: Some(u),
        },
        e => BenchError::Gpu(format!("cherenkov render: {e}")),
    }
}

/// The scene fill rule as the front-end's.
const fn front_rule(rule: cherenkov_scene::FillRule) -> cherenkov::FillRule {
    match rule {
        cherenkov_scene::FillRule::NonZero => cherenkov::FillRule::NonZero,
        cherenkov_scene::FillRule::EvenOdd => cherenkov::FillRule::EvenOdd,
    }
}

/// The shared lowering's specifics for this front end: `front_rule` fill
/// rules and the `this slice` interpolation api text.
const FRONT: convert::Front = convert::Front {
    engine: Cherenkov::NAME,
    fill_rule: front_rule,
    interpolation_api: Some("only srgb / linear interpolation in this slice"),
};

/// Applies a clip shape to a layer edit.
fn clip_shape(edit: &mut LayerEdit<Raster>, shape: &ShapeKind) {
    match shape {
        ShapeKind::Rect(r) => drop(edit.clip(*r)),
        ShapeKind::RoundedRect(r) => drop(edit.clip(*r)),
        ShapeKind::Continuous(c) => drop(edit.clip(*c)),
        ShapeKind::Circle(c) => drop(edit.clip(*c)),
        ShapeKind::Ellipse(e) => drop(edit.clip(*e)),
        ShapeKind::Line(l) => drop(edit.clip(*l)),
        ShapeKind::Path { path, .. } => drop(edit.clip(path.clone())),
    }
}

/// Registers every font a glyph run references, once per `(hash, index)`.
fn register_fonts(
    fonts: &mut HashMap<(ResourceHash, u32), cherenkov::Font>,
    engine: &CpuEngine<Raster>,
    layer: &SceneLayer,
    blobs: &Blobs,
) -> Result<(), BenchError> {
    for item in &layer.items {
        match item {
            Item::Layer(l) => register_fonts(fonts, engine, l, blobs)?,
            Item::Group(g) => register_group_fonts(fonts, engine, g, blobs)?,
            Item::Draw(SceneDraw::Glyphs(run)) => {
                if fonts.contains_key(&(run.font, run.font_index)) {
                    continue;
                }
                let blob = blobs
                    .get(&run.font)
                    .ok_or(cherenkov_scene::SceneError::MissingResource(run.font))?;
                let font = engine
                    .font(cherenkov::FontSource::bytes(blob.clone()).with_index(run.font_index))
                    .map_err(|e| match e {
                        ResourceError::Unsupported("color-font") => BenchError::Unsupported {
                            engine: Cherenkov::NAME,
                            feature: Feature::Glyphs,
                            api: Some(
                                "SVG colour fonts and non-PNG/BGRA bitmap formats are unsupported",
                            ),
                        },
                        e => BenchError::Engine(format!("cherenkov font: {e}")),
                    })?;
                fonts.insert((run.font, run.font_index), font);
            }
            Item::Draw(_) => {}
        }
    }
    Ok(())
}

/// [`register_fonts`] over a group's member list.
fn register_group_fonts(
    fonts: &mut HashMap<(ResourceHash, u32), cherenkov::Font>,
    engine: &CpuEngine<Raster>,
    group: &cherenkov_scene::Group,
    blobs: &Blobs,
) -> Result<(), BenchError> {
    for item in &group.items {
        match item {
            GroupItem::Group(g) => register_group_fonts(fonts, engine, g, blobs)?,
            GroupItem::Draw(SceneDraw::Glyphs(run)) => {
                if fonts.contains_key(&(run.font, run.font_index)) {
                    continue;
                }
                let blob = blobs
                    .get(&run.font)
                    .ok_or(cherenkov_scene::SceneError::MissingResource(run.font))?;
                let font = engine
                    .font(cherenkov::FontSource::bytes(blob.clone()).with_index(run.font_index))
                    .map_err(|e| match e {
                        ResourceError::Unsupported("color-font") => BenchError::Unsupported {
                            engine: Cherenkov::NAME,
                            feature: Feature::Glyphs,
                            api: Some(
                                "SVG colour fonts and non-PNG/BGRA bitmap formats are unsupported",
                            ),
                        },
                        e => BenchError::Engine(format!("cherenkov font: {e}")),
                    })?;
                fonts.insert((run.font, run.font_index), font);
            }
            GroupItem::Draw(_) => {}
        }
    }
    Ok(())
}

/// Keeps a registered image alive until the engine's surface drops —
/// `Image<F>` is format-typed, so the two encodings box separately.
#[expect(dead_code, reason = "the handles exist to keep uploads alive")]
enum ImageHandle {
    Rgba8(cherenkov::Image<cherenkov::Rgba8>),
    Rgba16F(cherenkov::Image<cherenkov::Rgba16F>),
}

/// Registers one scene image resource, once per (hash, encoding).
///
/// PNGs carry encoded sRGB or Display P3 data;
/// [`cherenkov_oracle::image::decode_png_rgba8`] is the shared decoder the
/// oracle and every adapter use, and the engine converts to the working
/// space at upload; `Rgba16F` blobs go through `Uploads<Rgba16F>` —
/// straight-alpha, already linear-light in the declared primaries.
fn register_image(
    images: &mut HashMap<(ResourceHash, ImageEncoding), cherenkov::ImageId>,
    handles: &mut Vec<ImageHandle>,
    engine: &CpuEngine<Raster>,
    hash: &ResourceHash,
    encoding: ImageEncoding,
    blobs: &Blobs,
) -> Result<(), BenchError> {
    let key = (*hash, encoding);
    if images.contains_key(&key) {
        return Ok(());
    }
    let blob = blobs
        .get(hash)
        .ok_or(cherenkov_scene::SceneError::MissingResource(*hash))?;
    let (width, height, rgba, color_space) = match encoding {
        ImageEncoding::Png { color_space } => {
            let (width, height, rgba) = cherenkov_oracle::image::decode_png_rgba8(blob)
                .map_err(|e| BenchError::Engine(format!("cherenkov image decode: {e}")))?;
            let space = match color_space {
                ImageColorSpace::Srgb => cherenkov::ImageColorSpace::Srgb,
                ImageColorSpace::DisplayP3 => cherenkov::ImageColorSpace::DisplayP3,
                _ => {
                    return Err(BenchError::Engine(format!(
                        "PNG images are sRGB-encoded, not {color_space:?}"
                    )));
                }
            };
            (width, height, rgba, space)
        }
        ImageEncoding::Rgba16F {
            width,
            height,
            color_space,
        } => {
            let expected = usize::try_from(width * height * 8)
                .map_err(|e| BenchError::Engine(format!("f16 image: {e}")))?;
            if blob.len() != expected {
                return Err(BenchError::Engine(format!(
                    "f16 image: {} bytes for {width}x{height}, expected {expected}",
                    blob.len()
                )));
            }
            let space = match color_space {
                ImageColorSpace::LinearSrgb => cherenkov::ImageColorSpace::LinearSrgb,
                ImageColorSpace::LinearP3 => cherenkov::ImageColorSpace::LinearP3,
                _ => {
                    return Err(BenchError::Engine(format!(
                        "f16 images are linear-light, not {color_space:?}"
                    )));
                }
            };
            let image = engine
                .image(
                    cherenkov::ImageData::<cherenkov::Rgba16F>::new(width, height, blob.clone())
                        .map_err(|e| BenchError::Engine(format!("cherenkov image: {e}")))?
                        .color_space(space),
                )
                .map_err(|e| BenchError::Engine(format!("cherenkov image: {e}")))?;
            images.insert(key, image.id());
            handles.push(ImageHandle::Rgba16F(image));
            return Ok(());
        }
    };
    let image = engine
        .image(
            cherenkov::ImageData::<cherenkov::Rgba8>::new(width, height, rgba)
                .map_err(|e| BenchError::Engine(format!("cherenkov image: {e}")))?
                .color_space(color_space),
        )
        .map_err(|e| BenchError::Engine(format!("cherenkov image: {e}")))?;
    images.insert(key, image.id());
    handles.push(ImageHandle::Rgba8(image));
    Ok(())
}

/// Registers every image referenced by draws or image paints in `layer`.
fn register_images(
    images: &mut HashMap<(ResourceHash, ImageEncoding), cherenkov::ImageId>,
    handles: &mut Vec<ImageHandle>,
    engine: &CpuEngine<Raster>,
    layer: &SceneLayer,
    blobs: &Blobs,
) -> Result<(), BenchError> {
    for item in &layer.items {
        match item {
            Item::Layer(l) => register_images(images, handles, engine, l, blobs)?,
            Item::Group(g) => register_group_images(images, handles, engine, g, blobs)?,
            Item::Draw(SceneDraw::Image {
                image, encoding, ..
            }) => {
                register_image(images, handles, engine, image, *encoding, blobs)?;
            }
            Item::Draw(d) => {
                let paint = match d {
                    SceneDraw::Fill { paint, .. } | SceneDraw::Stroke { paint, .. } => Some(paint),
                    SceneDraw::Glyphs(run) => Some(&run.paint),
                    _ => None,
                };
                if let Some(p) = paint.and_then(crate::convert::image_paint) {
                    register_image(images, handles, engine, &p.image, p.encoding, blobs)?;
                }
            }
        }
    }
    Ok(())
}

/// [`register_images`] over a group's member list.
fn register_group_images(
    images: &mut HashMap<(ResourceHash, ImageEncoding), cherenkov::ImageId>,
    handles: &mut Vec<ImageHandle>,
    engine: &CpuEngine<Raster>,
    group: &cherenkov_scene::Group,
    blobs: &Blobs,
) -> Result<(), BenchError> {
    for item in &group.items {
        match item {
            GroupItem::Group(g) => register_group_images(images, handles, engine, g, blobs)?,
            GroupItem::Draw(SceneDraw::Image {
                image, encoding, ..
            }) => {
                register_image(images, handles, engine, image, *encoding, blobs)?;
            }
            GroupItem::Draw(d) => {
                let paint = match d {
                    SceneDraw::Fill { paint, .. } | SceneDraw::Stroke { paint, .. } => Some(paint),
                    SceneDraw::Glyphs(run) => Some(&run.paint),
                    _ => None,
                };
                if let Some(p) = paint.and_then(crate::convert::image_paint) {
                    register_image(images, handles, engine, &p.image, p.encoding, blobs)?;
                }
            }
        }
    }
    Ok(())
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "scene filter parameters use f64; filtrate parameters use f32"
)]
fn register_filter(
    engine: &CpuEngine<Raster>,
    filter: &LayerFilter,
    blobs: &Blobs,
) -> Result<cherenkov::Filter, BenchError> {
    Ok(match filter {
        LayerFilter::ColorMatrix { matrix } => {
            engine.filter(filters::ColorMatrix(matrix.map(|value| value as f32)))
        }
        LayerFilter::ColorMatrixChain { first, second } => engine.filter(
            filters::ColorMatrix(first.map(|value| value as f32))
                .then(filters::ColorMatrix(second.map(|value| value as f32))),
        ),
        LayerFilter::GaussianBlur { sigma } => engine.filter(filters::GaussianBlur(*sigma as f32)),
        LayerFilter::BoxBlur { radius } => engine.filter(filters::Blur(*radius as f32)),
        LayerFilter::BlendImage {
            image,
            amount,
            mode,
        } => {
            let blob = blobs
                .get(image)
                .ok_or(cherenkov_scene::SceneError::MissingResource(*image))?;
            let (width, height, rgba) = cherenkov_oracle::image::decode_png_rgba8(blob)
                .map_err(|e| BenchError::Engine(format!("cherenkov filter image decode: {e}")))?;
            engine.filter(filters::BlendWithImage {
                image: FilterImage::from_rgba8(width, height, rgba),
                amount: *amount as f32,
                mode: filter_blend(*mode),
            })
        }
    })
}

const fn filter_blend(mode: FilterBlend) -> filters::BlendMode {
    match mode {
        FilterBlend::Normal => filters::BlendMode::Normal,
        FilterBlend::Multiply => filters::BlendMode::Multiply,
        FilterBlend::Screen => filters::BlendMode::Screen,
        FilterBlend::Overlay => filters::BlendMode::Overlay,
        FilterBlend::Darken => filters::BlendMode::Darken,
        FilterBlend::Lighten => filters::BlendMode::Lighten,
        FilterBlend::SoftLight => filters::BlendMode::SoftLight,
        FilterBlend::HardLight => filters::BlendMode::HardLight,
        FilterBlend::Difference => filters::BlendMode::Difference,
        FilterBlend::Exclusion => filters::BlendMode::Exclusion,
        FilterBlend::ColorDodge => filters::BlendMode::ColorDodge,
        FilterBlend::ColorBurn => filters::BlendMode::ColorBurn,
        FilterBlend::Hue => filters::BlendMode::Hue,
        FilterBlend::Saturation => filters::BlendMode::Saturation,
        FilterBlend::Color => filters::BlendMode::Color,
        FilterBlend::Luminosity => filters::BlendMode::Luminosity,
    }
}

/// Lowers a scene layer: one engine layer per scene layer, plus one per
/// draw run that must interleave with child layers.
fn prep_layer(
    layer: &SceneLayer,
    engine: &CpuEngine<Raster>,
    fonts: &HashMap<(ResourceHash, u32), cherenkov::Font>,
    images: &HashMap<(ResourceHash, ImageEncoding), cherenkov::ImageId>,
    blobs: &Blobs,
) -> Result<PrepLayer, BenchError> {
    // The engine draws a layer's content before its children, so the draws
    // are the layer's own content only when every draw precedes every child.
    // A group is recorded content like a draw: it scopes inside the layer.
    let first_child = layer.items.iter().position(|i| matches!(i, Item::Layer(_)));
    let last_draw = layer
        .items
        .iter()
        .rposition(|i| matches!(i, Item::Draw(_) | Item::Group(_)));
    let own = match (first_child, last_draw) {
        (None, _) | (Some(_), None) => true,
        (Some(f), Some(l)) => l < f,
    };
    let mut prep = PrepLayer {
        transform: layer.transform,
        scroll_offset: layer.scroll_offset,
        clip: layer
            .clip
            .as_ref()
            .map(|s| shape_kind(s, cherenkov::FillRule::NonZero)),
        opacity: layer.opacity,
        blend: engine_blend(layer.blend),
        filter: layer
            .filter
            .as_deref()
            .map(|filter| register_filter(engine, filter, blobs))
            .transpose()?,
        own: ContentRun {
            ops: Vec::new(),
            live: Vec::new(),
            motions: Vec::new(),
        },
        items: Vec::new(),
        // A `Motion::Paint` animates a content operand, not a layer
        // property — `paint_motion` binds it inside the content run.
        motion: match &layer.motion {
            Some(Motion::Paint { .. }) => None,
            motion => motion
                .as_ref()
                .map(|m| LayerMotion::from_scene(m, layer.transform, layer.projection.as_deref())),
        },
        projection: layer
            .projection
            .as_deref()
            .map(LayerProjection::from_scene)
            .transpose()?,
        backdrop: layer.backdrop,
        backdrop_effect: layer.backdrop_effect.clone(),
    };
    // A text layer records its source through the engine's parley
    // adapter; its items are the reference lowering the oracle draws.
    if let Some(text) = &layer.text {
        prep.own
            .ops
            .push(text_op(text, fonts, images, blobs, &FRONT)?);
        return Ok(prep);
    }
    if own {
        for (index, item) in layer.items.iter().enumerate() {
            match item {
                Item::Layer(l) => prep.items.push(PrepItem::Layer(Box::new(prep_layer(
                    l, engine, fonts, images, blobs,
                )?))),
                _ => prep_run_item(&mut prep.own, layer, index, item, fonts, images, blobs)?,
            }
        }
    } else {
        let mut run = ContentRun {
            ops: Vec::new(),
            live: Vec::new(),
            motions: Vec::new(),
        };
        for (index, item) in layer.items.iter().enumerate() {
            match item {
                Item::Layer(l) => {
                    if !run.ops.is_empty() {
                        prep.items.push(PrepItem::Content(std::mem::replace(
                            &mut run,
                            ContentRun {
                                ops: Vec::new(),
                                live: Vec::new(),
                                motions: Vec::new(),
                            },
                        )));
                    }
                    prep.items.push(PrepItem::Layer(Box::new(prep_layer(
                        l, engine, fonts, images, blobs,
                    )?)));
                }
                _ => prep_run_item(&mut run, layer, index, item, fonts, images, blobs)?,
            }
        }
        if !run.ops.is_empty() {
            prep.items.push(PrepItem::Content(run));
        }
    }
    Ok(prep)
}

/// Prepares a draw-or-group item of `layer` into `run`: pushes its op and
/// records any live entry or paint motion that targets it.
fn prep_run_item(
    run: &mut ContentRun,
    layer: &SceneLayer,
    index: usize,
    item: &Item,
    fonts: &HashMap<(ResourceHash, u32), cherenkov::Font>,
    images: &HashMap<(ResourceHash, ImageEncoding), cherenkov::ImageId>,
    blobs: &Blobs,
) -> Result<(), BenchError> {
    match item {
        Item::Draw(d) => {
            run.ops.push(op(d, fonts, images, blobs, &FRONT)?);
            let position = run.ops.len() - 1;
            if let Some(live) = live_run(layer, index, position, fonts, images, blobs)? {
                run.live.push(live);
            }
            if let Some(motion) = paint_motion(layer, index, position, &run.ops[position], images)?
            {
                run.motions.push(motion);
            }
        }
        Item::Group(g) => {
            run.ops.push(group_op(g, fonts, images, blobs, &FRONT)?);
        }
        Item::Layer(_) => unreachable!("layers are never run items"),
    }
    Ok(())
}

/// Resolves a `Motion::Paint` targeting item `index` into a
/// [`PaintMotion`] at `position` inside its content run. `None` when the
/// layer's motion is not a paint motion on this item. Errors when the
/// draw has no paint operand or is already a live item.
fn paint_motion(
    layer: &SceneLayer,
    index: usize,
    position: usize,
    op: &Op,
    images: &HashMap<(ResourceHash, ImageEncoding), cherenkov::ImageId>,
) -> Result<Option<PaintMotion>, BenchError> {
    let Some(Motion::Paint {
        item,
        from,
        animation,
    }) = &layer.motion
    else {
        return Ok(None);
    };
    if *item != index {
        return Ok(None);
    }
    if layer.live.iter().any(|live| live.item == index) {
        return Err(BenchError::Engine(
            "cherenkov: a live entry and a paint motion target one draw".into(),
        ));
    }
    let Some(target) = paint_op(op) else {
        return Err(BenchError::Engine(
            "cherenkov: a paint motion targets a draw without a paint".into(),
        ));
    };
    Ok(Some(PaintMotion {
        index: position,
        binding: nami::binding(convert::front_paint(from.as_ref(), images, &FRONT)?),
        target,
        animation: motion_animation(*animation),
    }))
}

/// Resolves a scene `live` entry targeting item `index` into a [`LiveRun`]
/// at `position` inside its content run. `None` when no entry targets it.
/// Errors when the target is not a draw or the frames are not all the
/// same draw variant as the item.
fn live_run(
    layer: &SceneLayer,
    index: usize,
    position: usize,
    fonts: &HashMap<(ResourceHash, u32), cherenkov::Font>,
    images: &HashMap<(ResourceHash, ImageEncoding), cherenkov::ImageId>,
    blobs: &Blobs,
) -> Result<Option<LiveRun>, BenchError> {
    let Some(entry) = layer.live.iter().find(|live| live.item == index) else {
        return Ok(None);
    };
    let Item::Draw(base) = &layer.items[index] else {
        return Err(BenchError::Engine(
            "cherenkov: a live entry does not target a draw item".into(),
        ));
    };
    let kind = std::mem::discriminant(&op(base, fonts, images, blobs, &FRONT)?);
    let mut frames = Vec::with_capacity(entry.frames.len());
    for draw in &entry.frames {
        let op = op(draw, fonts, images, blobs, &FRONT)?;
        if std::mem::discriminant(&op) != kind {
            return Err(BenchError::Engine(
                "cherenkov: a live frame is not the item's draw variant".into(),
            ));
        }
        frames.push(op);
    }
    Ok(Some(LiveRun {
        index: position,
        bindings: LiveBindings::for_frames(&frames),
        frames,
        previous: Some(0),
    }))
}

/// Builds one engine layer for `prep` under `parent`, recursing into
/// children in item order. The scene root maps onto the surface root, as
/// in the oracle, so a blended child of the scene root composites against
/// the surface clear colour; nested layers get their own engine layer.
#[expect(
    clippy::cast_possible_truncation,
    reason = "layer opacity is f32 at the engine boundary"
)]
fn build_layer(
    surface: &Surface<Raster>,
    tx: &mut Transaction<'_, Raster>,
    parent: Option<&CpuLayer>,
    prep: PrepLayer,
    groups: &HashMap<u32, cherenkov::BackdropGroup>,
    content_layers: &mut Vec<ContentLayer>,
    filter_handles: &mut Vec<cherenkov::Filter>,
) {
    let owned = parent.map(|_| surface.layer());
    let layer = owned.as_ref().unwrap_or_else(|| surface.root());
    {
        let edit = &mut tx[layer];
        edit.transform(prep.transform);
        if let Some(projection) = &prep.projection {
            projection.apply(edit);
        }
        edit.scroll_offset(prep.scroll_offset);
        edit.opacity(prep.opacity as f32);
        edit.blend(prep.blend);
        if let Some(filter) = &prep.filter {
            edit.filter(filter.id());
        }
        if let Some(clip) = &prep.clip {
            clip_shape(edit, clip);
        }
        if let Some(id) = prep.backdrop {
            let group = &groups[&id];
            match &prep.backdrop_effect {
                None => {
                    edit.backdrop(group.sample());
                }
                Some(spec) => {
                    use BackdropEffectSpec as S;
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "effect parameters are f32 at the engine boundary"
                    )]
                    let effect: cherenkov::BackdropEffect = match spec {
                        S::ColorMatrix { matrix } => {
                            cherenkov::ColorMatrix(matrix.map(|v| v as f32)).into()
                        }
                        S::Refraction { depth, strength } => cherenkov::Refraction {
                            depth: *depth as f32,
                            strength: *strength as f32,
                        }
                        .into(),
                        S::RimLight { width, color, gain } => cherenkov::Rim {
                            width: *width as f32,
                            color: color.map(|v| v as f32),
                            gain: *gain as f32,
                        }
                        .into(),
                    };
                    edit.backdrop(group.sample_with(effect));
                }
            }
        }
    }
    if let Some(parent) = parent {
        tx[parent].push(layer);
    }
    for item in prep.items {
        match item {
            PrepItem::Content(run) => {
                let child = surface.layer();
                tx[layer].push(&child);
                content_layers.push(ContentLayer {
                    layer: Some(child),
                    ops: run.ops,
                    live: run.live,
                    motions: run.motions,
                    motion: None,
                });
            }
            PrepItem::Layer(p) => {
                build_layer(
                    surface,
                    tx,
                    Some(layer),
                    *p,
                    groups,
                    content_layers,
                    filter_handles,
                );
            }
        }
    }
    if let Some(filter) = prep.filter {
        filter_handles.push(filter);
    }
    content_layers.push(ContentLayer {
        layer: owned,
        ops: prep.own.ops,
        live: prep.own.live,
        motions: prep.own.motions,
        motion: prep.motion,
    });
}

/// Creates the engine backdrop group for a scene group. The chain type is
/// static, so the combinations this adapter builds are a blur alone, a
/// colour matrix alone, and a blur then a colour matrix (the shapes the
/// corpus uses); anything else is reported unsupported rather than
/// approximated.
#[expect(
    clippy::cast_possible_truncation,
    reason = "filter parameters are f32 at the engine boundary"
)]
fn backdrop_group(
    surface: &Surface<Raster>,
    group: &cherenkov_scene::BackdropGroup,
) -> Result<cherenkov::BackdropGroup, BenchError> {
    use filtrate::filters::{ColorMatrix, GaussianBlur};
    let unsupported = || BenchError::Unsupported {
        engine: Cherenkov::NAME,
        feature: Feature::Backdrop,
        api: Some("backdrop filter chain shape is not built"),
    };
    let scale = convert::capture_scale(group)?;
    Ok(match group.filters.as_slice() {
        [] => surface.backdrop_group_unfiltered(scale),
        [BackdropFilter::GaussianBlur { sigma }] => {
            surface.backdrop_group(GaussianBlur(*sigma as f32), scale)
        }
        [BackdropFilter::ColorMatrix { matrix }] => {
            surface.backdrop_group(ColorMatrix(matrix.map(|v| v as f32)), scale)
        }
        [
            BackdropFilter::GaussianBlur { sigma },
            BackdropFilter::ColorMatrix { matrix },
        ] => surface.backdrop_group(
            GaussianBlur(*sigma as f32).then(ColorMatrix(matrix.map(|v| v as f32))),
            scale,
        ),
        _ => return Err(unsupported()),
    })
}

/// The engine and prepared scene state do not format.
impl std::fmt::Debug for Cherenkov {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cherenkov")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl Cherenkov {
    /// Adapter key.
    pub const NAME: &'static str = "cherenkov-cpu";

    /// The readback format `CHERENKOV_CPU_READBACK` selects (`f16` default,
    /// `f32` for the unrounded framebuffer).
    fn readback_format() -> OffscreenFormat {
        match std::env::var("CHERENKOV_CPU_READBACK").as_deref() {
            Ok("f32") => OffscreenFormat::LinearF32,
            _ => OffscreenFormat::LinearF16,
        }
    }

    /// Creates the adapter, initializing the raster engine.
    ///
    /// # Errors
    /// [`BenchError::Gpu`] when the render thread or worker pool fails.
    pub fn new() -> Result<Self, BenchError> {
        let engine = CpuEngine::<Raster>::new(RasterConfig::default())
            .map_err(|e| BenchError::Gpu(format!("cherenkov engine: {e}")))?;
        Ok(Self {
            info: EngineInfo {
                name: Self::NAME,
                engine_crate: "cherenkov-cpu",
                crate_version: env!("DEP_CHERENKOV_CPU_VERSION"),
                source_rev: option_env!("DEP_CHERENKOV_CPU_SOURCE_REV").map(String::from),
                output_format: match Self::readback_format() {
                    OffscreenFormat::LinearF16 => {
                        "f16 framebuffer, rounded once at band emit (premultiplied linear P3)"
                    }
                    OffscreenFormat::LinearF32 => {
                        "f32 framebuffer, unrounded (premultiplied linear P3)"
                    }
                }
                .to_string(),
                precision: "f32 exact-area coverage bands; f32 band scratch, output-format framebuffer",
                route: "cpu-raster (rayon bands)",
                color_note: "premultiplied linear Display P3 end to end; HDR channels unclamped",
                encode_scope: "records `cherenkov::Content` calls (fill/stroke/shadow/glyphs) \
                               against fonts and the layer tree prepared once",
            },
            engine,
            surface: None,
            fonts: HashMap::new(),
            images: HashMap::new(),
            image_handles: Vec::new(),
            filter_handles: Vec::new(),
            backdrop_groups: HashMap::new(),
            content_layers: Vec::new(),
            has_motion: false,
            motion_committed: false,
            has_live: false,
            frame: 0,
            clock: Clock::new(),
            timings: Timings::default(),
            counters: Counters::default(),
            present: None,
            headroom: 1.0,
        })
    }
}

impl Engine for Cherenkov {
    fn info(&self) -> &EngineInfo {
        &self.info
    }

    fn supported(&self) -> BTreeSet<Feature> {
        cherenkov_features().into_iter().collect()
    }

    fn prepare(&mut self, input: &EncodeInput<'_>) -> Result<(), BenchError> {
        convert::check_features(Self::NAME, input.scene, &cherenkov_features(), |f| {
            convert::missing_api(&FRONT, f)
        })?;
        // sRGB presentation reads the native f32 framebuffer; the
        // linear-P3 destination stores f16, modelled by the f16 readback.
        #[expect(
            clippy::cast_possible_truncation,
            reason = "Display::headroom is f32 at the engine boundary"
        )]
        {
            self.headroom = input.scene.present_headroom as f32;
        }
        let format = match self.present {
            Some(PresentKind::SrgbHw | PresentKind::SrgbShader) => OffscreenFormat::LinearF32,
            Some(PresentKind::LinearP3) => OffscreenFormat::LinearF16,
            Some(kind) => {
                return Err(BenchError::Engine(format!(
                    "cherenkov-cpu: {} presentation is unsupported",
                    kind.name()
                )));
            }
            None => Self::readback_format(),
        };
        let surface = self
            .engine
            .surface(Offscreen::new(
                (input.scene.width, input.scene.height),
                format,
            ))
            .map_err(|e| BenchError::Gpu(format!("cherenkov surface: {e}")))?;
        surface.clear_color(working(&input.scene.clear));
        register_fonts(
            &mut self.fonts,
            &self.engine,
            &input.scene.root,
            input.blobs,
        )?;
        register_images(
            &mut self.images,
            &mut self.image_handles,
            &self.engine,
            &input.scene.root,
            input.blobs,
        )?;
        let prep = prep_layer(
            &input.scene.root,
            &self.engine,
            &self.fonts,
            &self.images,
            input.blobs,
        )?;
        self.content_layers.clear();
        self.backdrop_groups.clear();
        let mut backdrop_groups = HashMap::new();
        for group in &input.scene.backdrop_groups {
            backdrop_groups.insert(group.id, backdrop_group(&surface, group)?);
        }
        let mut content_layers = Vec::new();
        let mut filter_handles = Vec::new();
        surface.update(|tx| {
            build_layer(
                &surface,
                tx,
                None,
                prep,
                &backdrop_groups,
                &mut content_layers,
                &mut filter_handles,
            );
        });
        self.backdrop_groups = backdrop_groups;
        self.has_motion = content_layers
            .iter()
            .any(|c| c.motion.is_some() || !c.motions.is_empty());
        self.motion_committed = false;
        self.has_live = content_layers.iter().any(|c| !c.live.is_empty());
        self.frame = 0;
        self.content_layers = content_layers;
        self.filter_handles = filter_handles;
        self.surface = Some(surface);
        Ok(())
    }

    fn encode(&mut self, input: &EncodeInput<'_>) -> Result<(), BenchError> {
        self.counters = Counters::default();
        convert::count_layer(&input.scene.root, &mut self.counters);
        let surface = self
            .surface
            .as_ref()
            .ok_or_else(|| BenchError::Engine("cherenkov: encode before prepare".into()))?;
        let first_frame = self.frame == 0;
        if self.has_motion && first_frame {
            for cl in &self.content_layers {
                if let Some(motion) = &cl.motion {
                    motion.apply(surface, cl.handle(surface));
                }
            }
            self.motion_committed = true;
        }
        // Motion and live scenes record their content once: later encodes
        // only set live bindings and advance the clock. Static scenes keep
        // re-recording each frame so their numbers stay comparable.
        if first_frame || !(self.has_motion || self.has_live) {
            surface.update(|tx| {
                if self.has_motion {
                    for cl in &self.content_layers {
                        tx[cl.handle(surface)]
                            .record(|c| record_ops(c, &cl.ops, &cl.live, &cl.motions));
                    }
                } else {
                    for cl in &self.content_layers {
                        // A motionless scene's record keeps the dev shape:
                        // no motions reach it at all.
                        tx[cl.handle(surface)].record(|c| record_ops_static(c, &cl.ops, &cl.live));
                    }
                }
            });
            // A paint motion starts once the content carrying its binding
            // is installed: the `set` lands the animated commit, and the
            // first sample shows `from`.
            if first_frame {
                for cl in &self.content_layers {
                    for motion in &cl.motions {
                        motion.binding.set(motion.target.clone());
                    }
                }
            }
        } else {
            for cl in &mut self.content_layers {
                for live in &mut cl.live {
                    live.advance(self.frame);
                }
            }
        }
        self.frame += 1;
        self.clock.advance();
        Ok(())
    }

    /// Puts the adapter into presentation mode: `submit`'s readback is
    /// run through the backend's presentation and lifted back to the
    /// working space, matching the GPU adapter. The CPU backend presents
    /// sRGB and extended linear P3 only — the #98 encoded destinations
    /// are swapchain colour spaces a CPU framebuffer does not model.
    fn present(&mut self, kind: PresentKind) -> Result<(), BenchError> {
        match kind {
            PresentKind::SrgbHw | PresentKind::SrgbShader | PresentKind::LinearP3 => {}
            _ => {
                return Err(BenchError::Engine(format!(
                    "cherenkov-cpu: {} presentation is unsupported",
                    kind.name()
                )));
            }
        }
        self.present = Some(kind);
        Ok(())
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "working-space pixels fit f32"
    )]
    fn submit(&mut self, frame: u64, readback: bool) -> Result<Submit, BenchError> {
        if self.surface.is_none() {
            return Err(BenchError::Engine(
                "cherenkov: submit before prepare".into(),
            ));
        }
        let render_at = std::time::Instant::now();
        self.timings.render_frame(
            &self.engine,
            &mut self.clock,
            frame,
            readback && self.has_motion,
            render_error,
        )?;
        let render_seconds = render_at.elapsed().as_secs_f64();
        let readback_at = std::time::Instant::now();
        let image = if readback {
            let rb = self
                .surface
                .as_ref()
                .expect("checked above")
                .readback()
                .map_err(render_error)?;
            let pixels = match self.present {
                // sRGB presentation: the framebuffer goes through the
                // backend's present path into encoded premultiplied
                // sRGB bytes, then lifts back to the working space for
                // the comparison — the same interchange the GPU adapter
                // uses on its presented texture.
                Some(PresentKind::SrgbHw | PresentKind::SrgbShader) => {
                    cherenkov_cpu::present_srgb8(self.headroom, &rb.pixels)
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|texel| {
                            let encoded = texel.map(|v| f64::from(v) / 255.0);
                            let p3 = presented_srgb_to_working(encoded);
                            [p3[0] as f32, p3[1] as f32, p3[2] as f32, p3[3] as f32]
                        })
                        .collect()
                }
                // Extended linear Display P3 tone-maps to the display
                // headroom, like the GPU's `LinearP3` destination.
                Some(PresentKind::LinearP3) => {
                    cherenkov_cpu::present_linear_p3(self.headroom, &rb.pixels)
                }
                Some(kind) => {
                    return Err(BenchError::Engine(format!(
                        "cherenkov-cpu: {} presentation is unsupported",
                        kind.name()
                    )));
                }
                None => rb.pixels,
            };
            Some(cherenkov_oracle::F32Image {
                width: rb.width,
                height: rb.height,
                pixels,
            })
        } else {
            None
        };
        Ok(Submit {
            image,
            gpu: Vec::new(),
            phases: None,
            render_seconds: Some(render_seconds),
            readback_seconds: readback.then(|| readback_at.elapsed().as_secs_f64()),
        })
    }

    fn finish_gpu(&mut self) -> Result<Vec<GpuSample>, BenchError> {
        let timings = self.engine.finish_timings().map_err(render_error)?;
        Ok(self.timings.samples(timings))
    }

    fn counters(&self) -> Counters {
        let mut counters = self.counters.clone();
        let stats = self.engine.stats();
        let memory = self.engine.memory();
        counters.dispatches = Some(stats.draws);
        counters.passes = Some(stats.passes);
        counters.memory_gpu_bytes = Some(memory.gpu.0);
        counters.memory_cpu_bytes = Some(memory.cpu.0);
        counters.memory_backdrop_capture_bytes = Some(memory.backdrop_captures.0);
        counters.memory_backdrop_capture_format = memory.backdrop_capture_format;
        counters
    }

    fn trim(&mut self) -> Result<(), BenchError> {
        self.engine.trim(cherenkov::Pressure::Moderate);
        Ok(())
    }

    fn device(&self) -> DeviceInfo {
        let info = self.engine.info();
        DeviceInfo {
            adapter: Some(format!("cherenkov-cpu ({} threads)", info.threads)),
            backend: Some(info.simd.to_string()),
            driver: None,
            driver_info: None,
            vendor: None,
            device: None,
            target_format: Some("f32 RGBA framebuffer".to_string()),
            cpu: info.cpu.clone().or_else(crate::cpu_model),
            thermal_celsius: crate::thermal_celsius(),
        }
    }

    fn memory(&self) -> AdapterMemory {
        let usage = self.engine.memory();
        AdapterMemory {
            engine: Reading::Measured(EngineBytes {
                cpu_bytes: usage.cpu.0,
                gpu_bytes: usage.gpu.0,
                backdrop_capture_bytes: usage.backdrop_captures.0,
            }),
            wgpu_allocator: Reading::unavailable("cherenkov-cpu has no wgpu allocator"),
            skia_budgeted: Reading::unavailable("cherenkov-cpu has no Skia budget"),
            vk_memory_budget: Reading::unavailable("cherenkov-cpu has no Vulkan device"),
        }
    }
}

/// Records one [`Op`] into a recorder — the per-frame engine calls.
#[expect(
    clippy::inline_always,
    reason = "the record closure must keep dev's per-op call-free codegen"
)]
#[inline(always)]
fn record_op(c: &mut cherenkov::Recorder, op: &Op) {
    match op {
        Op::Image {
            image,
            dst,
            sampling,
        } => c.image(*image, Fixed(*dst), *sampling),
        Op::Fill { shape, paint, .. } => match shape {
            ShapeKind::Rect(s) => c.fill(Fixed(*s), Fixed(paint.clone())),
            ShapeKind::RoundedRect(s) => {
                c.fill(Fixed(*s), Fixed(paint.clone()));
            }
            ShapeKind::Continuous(s) => {
                c.fill(Fixed(*s), Fixed(paint.clone()));
            }
            ShapeKind::Circle(s) => c.fill(Fixed(*s), Fixed(paint.clone())),
            ShapeKind::Ellipse(s) => c.fill(Fixed(*s), Fixed(paint.clone())),
            ShapeKind::Line(s) => c.fill(Fixed(*s), Fixed(paint.clone())),
            ShapeKind::Path { data, .. } => c.fill(Fixed(data.clone()), Fixed(paint.clone())),
        },
        Op::Stroke {
            shape,
            stroke,
            paint,
        } => match shape {
            ShapeKind::Rect(s) => c.stroke(Fixed(*s), Fixed(stroke.clone()), Fixed(paint.clone())),
            ShapeKind::RoundedRect(s) => {
                c.stroke(Fixed(*s), Fixed(stroke.clone()), Fixed(paint.clone()));
            }
            ShapeKind::Continuous(s) => {
                c.stroke(Fixed(*s), Fixed(stroke.clone()), Fixed(paint.clone()));
            }
            ShapeKind::Circle(s) => {
                c.stroke(Fixed(*s), Fixed(stroke.clone()), Fixed(paint.clone()));
            }
            ShapeKind::Ellipse(s) => {
                c.stroke(Fixed(*s), Fixed(stroke.clone()), Fixed(paint.clone()));
            }
            ShapeKind::Line(s) => c.stroke(Fixed(*s), Fixed(stroke.clone()), Fixed(paint.clone())),
            ShapeKind::Path { data, .. } => c.stroke(
                Fixed(data.clone()),
                Fixed(stroke.clone()),
                Fixed(paint.clone()),
            ),
        },
        Op::Shadow { shape, shadow } => match shape {
            ShapeKind::Rect(s) => c.shadow(Fixed(*s), Fixed(*shadow)),
            ShapeKind::RoundedRect(s) => c.shadow(Fixed(*s), Fixed(*shadow)),
            ShapeKind::Continuous(s) => c.shadow(Fixed(*s), Fixed(*shadow)),
            ShapeKind::Circle(s) => c.shadow(Fixed(*s), Fixed(*shadow)),
            ShapeKind::Ellipse(s) => c.shadow(Fixed(*s), Fixed(*shadow)),
            ShapeKind::Line(s) => c.shadow(Fixed(*s), Fixed(*shadow)),
            ShapeKind::Path { data, .. } => {
                c.shadow(Fixed(data.clone()), Fixed(*shadow));
            }
        },
        Op::Glyphs { run, paint } => c.glyphs(Fixed(run.clone()), Fixed(paint.clone())),
        Op::Group { group, ops } => c.group(Fixed(*group), |c| {
            for op in ops {
                record_op(c, op);
            }
        }),
        Op::Text { layout, origin } => cherenkov::draw_text(c, layout, *origin),
    }
}
