//! `cherenkov` adapter: the `cherenkov-gpu` backend slice on `wgpu`.
//!
//! Route: the front-end records a display list per content layer, lowered on
//! the render thread into retained analytic f32 quads drawn by specialized WGSL pipelines
//! into a `Rgba16Float` texture (premultiplied linear Display P3 — the
//! suite's working space end to end, so readback needs no conversion). GPU
//! time comes from real `wgpu` timestamps collected from completed earlier
//! submissions inside [`cherenkov::Engine::render`] with `GpuConfig::timestamps` set.
//!
//! `CHERENKOV_SCRATCH_FORMAT=rgba8` selects `Rgba8Unorm` isolation targets
//! (default `Rgba16Float`) to compare intermediate precision/bandwidth.

use std::collections::{BTreeSet, HashMap};

use cherenkov::{
    Draw as _, Engine as GpuEngine, Fixed, ImageData, Layer as GpuLayer, LayerEdit, Offscreen,
    OffscreenFormat, RenderError, ResourceError, Rgba8, Rgba16F, Surface, Transaction,
};
use cherenkov_gpu::interop::{
    OutputAlpha, OutputColor, Presenter, SharedDevice, TextureOutput, TextureTarget,
    shader_delivery, wgpu,
};
use cherenkov_gpu::{Gpu, GpuConfig, ScratchFormat};
use cherenkov_oracle::present::presented_srgb_to_working;
use cherenkov_scene::{
    BackdropFilter, BlendMode, BlendSpace, ColorSpace, Draw as SceneDraw, Feature, FilterBlend,
    GroupItem, ImageColorSpace, ImageEncoding, Item, Layer as SceneLayer, LayerFilter, Motion,
    ResourceHash,
};
use filtrate::{FilterExt, FilterImage, filters};
use kurbo::{Affine, BezPath, Circle, Ellipse, Line, Rect, RoundedRect, Vec2};

use crate::convert::{
    self, Blobs, Op, ShapeKind, engine_blend, group_op, op, shape_kind, text_op, working,
};
use crate::memory::{AdapterMemory, EngineBytes, Reading, wgpu_allocator, wgpu_vk_memory_budget};
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
    /// The live operand for `shape` under `rule` (paths only).
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

/// The core fill rule a scene rule lowers to.
const fn core_rule(rule: cherenkov_scene::FillRule) -> cherenkov::FillRule {
    match rule {
        cherenkov_scene::FillRule::EvenOdd => cherenkov::FillRule::EvenOdd,
        cherenkov_scene::FillRule::NonZero => cherenkov::FillRule::NonZero,
    }
}

/// The shared lowering's specifics for this front end: `core_rule` fill
/// rules and the `first slice` interpolation api text.
const FRONT: convert::Front = convert::Front {
    engine: Cherenkov::NAME,
    fill_rule: core_rule,
    interpolation_api: Some("only srgb / linear interpolation in the first slice"),
};

/// `op`'s shape operand, or `None` when it has none.
fn shape_op(op: &Op) -> Option<LiveShape> {
    match op {
        Op::Fill { shape, rule, .. } => Some(LiveShape::of(shape, core_rule(*rule))),
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
    /// Image destination.
    dst: Option<nami::Binding<Rect>>,
}

impl LiveBindings {
    /// Binds the operands that differ across `frames`.
    fn for_frames(frames: &[Op]) -> Self {
        let mut b = Self::default();
        let Some(base) = frames.first() else {
            return b;
        };
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
        if frames.iter().any(|f| dst_op(f) != dst_op(base)) {
            b.dst = dst_op(base).map(nami::binding);
        }
        b
    }

    /// Sets each bound operand to `op`'s value where it differs from `prev`.
    fn set(&self, op: &Op, prev: Option<&Op>) {
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
        if let Some(b) = &self.dst
            && prev.is_none_or(|p| dst_op(p) != dst_op(op))
        {
            b.set(dst_op(op).expect("bound op has a dst"));
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
        Op::Fill { shape, rule, paint } => c.fill(
            live_or_const(
                bindings.shape.as_ref(),
                &LiveShape::of(shape, core_rule(*rule)),
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
        Op::Image {
            image,
            dst,
            sampling,
        } => c.image(*image, live_or_const(bindings.dst.as_ref(), dst), *sampling),
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
                nami::constant(LiveShape::of(shape, core_rule(*rule))),
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
    /// The backdrop group this layer samples, if any.
    backdrop: Option<u32>,
    /// The member's per-member backdrop effect, if any.
    backdrop_effect: Option<cherenkov::BackdropEffect>,
    /// The member's outer extent in device pixels (`0` for none).
    backdrop_outer: cherenkov::BackdropOuter,
    /// The layer's projective pose.
    projection: Option<LayerProjection>,
    /// The layer's one-time motion.
    motion: Option<LayerMotion>,
}

/// An engine layer plus the ops it records each frame.
struct ContentLayer {
    /// The layer handle; `None` for the surface root.
    layer: Option<GpuLayer>,
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
    fn handle<'a>(&'a self, surface: &'a Surface<Gpu>) -> &'a GpuLayer {
        self.layer.as_ref().unwrap_or_else(|| surface.root())
    }
}

/// `cherenkov-gpu` adapter.
pub struct Cherenkov {
    info: EngineInfo,
    engine: GpuEngine<Gpu>,
    shared_device: SharedDevice,
    surface: Option<Surface<Gpu>>,
    /// Registered fonts per `(blob hash, face index)`.
    fonts: HashMap<(ResourceHash, u32), cherenkov::Font>,
    /// Registered images per (blob hash, declared encoding).
    images: HashMap<(ResourceHash, ImageEncoding), cherenkov::ImageId>,
    /// The `Image` handles keeping `images` registered.
    image_handles: Vec<ImageHandle>,
    /// Registered filters referenced by prepared layers.
    filter_handles: Vec<cherenkov::Filter>,
    /// Layers holding recorded content, in draw order.
    content_layers: Vec<ContentLayer>,
    /// Whether any layer carries a `motion`.
    has_motion: bool,
    /// Whether the motion commits have been sent (first encode).
    motion_committed: bool,
    /// The backdrop groups created in `prepare`, alive while the surface
    /// is (dropping one fails frames that still sample it).
    backdrop_groups: HashMap<u32, cherenkov::BackdropGroup>,
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
    /// `--present` mode: the shared device, the presentation pass and the
    /// per-scene source/destination textures. `None` renders offscreen.
    /// Boxed so the mode stays off the hot struct.
    present: Option<Box<Present>>,
    /// The #169 allocation diagnostic sink, when the run installed one.
    alloc_diag: Option<cherenkov_gpu::diag::Sink>,
}

/// `--present` state: the engine runs on a bench-owned shared device so a
/// render's working-space texture can be presented into the kind's
/// destination and read back.
struct Present {
    /// The `--present` kind in force.
    kind: PresentKind,
    /// The real presentation pass (`present.wgsl`).
    presenter: Presenter,
    /// The prepared surface's working-space texture (`Rgba16Float`),
    /// delivered by its `TextureTarget` channel.
    source: Option<wgpu::Texture>,
    /// The kind's destination texture, sized to the prepared scene.
    destination: Option<wgpu::Texture>,
    /// The scene's declared display headroom for `read`'s present pass.
    headroom: f32,
}

/// The destination texture format of a `--present` kind.
pub(crate) const fn present_format(kind: PresentKind) -> wgpu::TextureFormat {
    match kind {
        PresentKind::SrgbHw | PresentKind::DisplayP3Hw => wgpu::TextureFormat::Rgba8UnormSrgb,
        PresentKind::SrgbShader | PresentKind::DisplayP3Shader => wgpu::TextureFormat::Rgba8Unorm,
        PresentKind::LinearP3
        | PresentKind::Scrgb
        | PresentKind::ExtendedSrgb
        | PresentKind::ExtendedP3
        | PresentKind::Pq
        | PresentKind::Hlg => wgpu::TextureFormat::Rgba16Float,
    }
}

/// Bytes per pixel of a kind's destination texture.
const fn present_texel_size(kind: PresentKind) -> u32 {
    match kind {
        PresentKind::SrgbHw
        | PresentKind::SrgbShader
        | PresentKind::DisplayP3Hw
        | PresentKind::DisplayP3Shader => 4,
        PresentKind::LinearP3
        | PresentKind::Scrgb
        | PresentKind::ExtendedSrgb
        | PresentKind::ExtendedP3
        | PresentKind::Pq
        | PresentKind::Hlg => 8,
    }
}

/// The [`TextureOutput::color`] of a `--present` kind.
pub(crate) const fn present_color(kind: PresentKind) -> OutputColor {
    match kind {
        PresentKind::SrgbHw | PresentKind::SrgbShader => OutputColor::Srgb,
        PresentKind::LinearP3 => OutputColor::LinearDisplayP3,
        PresentKind::DisplayP3Hw | PresentKind::DisplayP3Shader => OutputColor::DisplayP3,
        PresentKind::Scrgb => OutputColor::ExtendedSrgbLinear,
        PresentKind::ExtendedSrgb => OutputColor::ExtendedSrgb,
        PresentKind::ExtendedP3 => OutputColor::ExtendedDisplayP3,
        PresentKind::Pq => OutputColor::Bt2100Pq,
        PresentKind::Hlg => OutputColor::Bt2100Hlg,
    }
}

/// The `--present` destination texture.
fn present_target(device: &wgpu::Device, size: (u32, u32), kind: PresentKind) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bench presentation"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: present_format(kind),
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// Copies the destination texture to a mapped buffer and returns its
/// texels.
fn read_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    texel: u32,
) -> Result<Vec<u8>, BenchError> {
    let (w, h) = (texture.width(), texture.height());
    let bytes_per_row = (w * texel).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("presentation readback"),
        size: u64::from(bytes_per_row) * u64::from(h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("presentation readback"),
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    let submission = queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .map_err(|e| BenchError::Gpu(format!("presentation readback wait: {e}")))?;
    receive
        .recv()
        .map_err(|e| BenchError::Gpu(format!("presentation readback: {e}")))?
        .map_err(|e| BenchError::Gpu(format!("presentation readback map: {e}")))?;
    let data = buffer
        .slice(..)
        .get_mapped_range()
        .expect("buffer range is mapped and not overlapping");
    let row = (w * texel) as usize;
    let mut packed = Vec::with_capacity(row * h as usize);
    for y in 0..h as usize {
        packed
            .extend_from_slice(&data[y * bytes_per_row as usize..y * bytes_per_row as usize + row]);
    }
    drop(data);
    buffer.unmap();
    Ok(packed)
}

/// Decodes the presented destination texels into the working-space
/// interchange image. sRGB kinds hold encoded premultiplied sRGB bytes —
/// the displayed colour — lifted back to linear P3 so both sides of the
/// comparison live in the working space. `linear-p3` is the f16
/// working-space texel verbatim.
#[expect(
    clippy::cast_possible_truncation,
    reason = "u8 texels decode through f64 into the f32 interchange image"
)]
fn presented_pixels(
    kind: PresentKind,
    width: u32,
    height: u32,
    packed: &[u8],
) -> cherenkov_oracle::F32Image {
    let mut pixels = Vec::with_capacity((width * height) as usize);
    match kind {
        PresentKind::SrgbHw | PresentKind::SrgbShader => {
            for texel in packed.as_chunks::<4>().0 {
                let encoded = texel.map(|v| f64::from(v) / 255.0);
                let p3 = presented_srgb_to_working(encoded);
                pixels.push([p3[0] as f32, p3[1] as f32, p3[2] as f32, p3[3] as f32]);
            }
        }
        PresentKind::DisplayP3Hw | PresentKind::DisplayP3Shader => {
            for texel in packed.as_chunks::<4>().0 {
                let encoded = texel.map(|v| f64::from(v) / 255.0);
                let p3 = cherenkov_oracle::present::presented_display_p3_to_working(encoded);
                pixels.push([p3[0] as f32, p3[1] as f32, p3[2] as f32, p3[3] as f32]);
            }
        }
        PresentKind::LinearP3
        | PresentKind::Scrgb
        | PresentKind::ExtendedSrgb
        | PresentKind::ExtendedP3
        | PresentKind::Pq
        | PresentKind::Hlg => {
            for texel in packed.as_chunks::<8>().0 {
                let mut p = [0.0; 4];
                for (c, b) in p.iter_mut().zip(texel.as_chunks::<2>().0) {
                    *c = half::f16::from_bits(u16::from_le_bytes(*b)).to_f32();
                }
                let p = match kind {
                    PresentKind::LinearP3 => p,
                    PresentKind::Scrgb => {
                        cherenkov_oracle::present::presented_extended_srgb_to_working(
                            false,
                            p.map(f64::from),
                        )
                        .map(|c| c as f32)
                    }
                    PresentKind::ExtendedSrgb => {
                        cherenkov_oracle::present::presented_extended_srgb_to_working(
                            true,
                            p.map(f64::from),
                        )
                        .map(|c| c as f32)
                    }
                    PresentKind::ExtendedP3 => {
                        cherenkov_oracle::present::presented_extended_p3_to_working(
                            p.map(f64::from),
                        )
                        .map(|c| c as f32)
                    }
                    PresentKind::Pq => {
                        cherenkov_oracle::present::presented_pq_to_working(p.map(f64::from))
                            .map(|c| c as f32)
                    }
                    PresentKind::Hlg => {
                        cherenkov_oracle::present::presented_hlg_to_working(p.map(f64::from))
                            .map(|c| c as f32)
                    }
                    _ => unreachable!("matched above"),
                };
                pixels.push(p);
            }
        }
    }
    cherenkov_oracle::F32Image {
        width,
        height,
        pixels,
    }
}

impl Present {
    /// Presents the prepared scene's working-space texture into the
    /// kind's destination on `shared`'s device and queue and reads it back.
    fn read(&mut self, shared: &SharedDevice) -> Result<cherenkov_oracle::F32Image, BenchError> {
        let source = self
            .source
            .as_ref()
            .ok_or_else(|| BenchError::Engine("present before prepare".into()))?;
        let destination = self.destination.as_ref().expect("set at prepare");
        self.presenter.texture(
            &shared.device,
            &shared.queue,
            &source.create_view(&wgpu::TextureViewDescriptor::default()),
            TextureOutput {
                texture: destination,
                color: present_color(self.kind),
                alpha: OutputAlpha::Premultiplied,
                headroom: self.headroom,
            },
        );
        let packed = read_texture(
            &shared.device,
            &shared.queue,
            destination,
            present_texel_size(self.kind),
        )?;
        Ok(presented_pixels(
            self.kind,
            destination.width(),
            destination.height(),
            &packed,
        ))
    }
}

/// The features this slice executes faithfully.
fn cherenkov_features() -> Vec<Feature> {
    vec![
        Feature::Fill,
        Feature::Stroke,
        Feature::ContinuousCorners,
        Feature::PaintTransform,
        Feature::LinearGradient,
        Feature::RadialGradient,
        Feature::Clip,
        Feature::Opacity,
        Feature::Filter,
        Feature::Shadow,
        Feature::Glyphs,
        Feature::GlyphStroke,
        Feature::GlyphTransform,
        Feature::FontVariations,
        Feature::Scroll,
        Feature::Animation,
        Feature::HdrColor,
        Feature::WideGamut,
        Feature::Path,
        Feature::EvenOdd,
        Feature::StrokeDash,
        Feature::SweepGradient,
        Feature::MeshGradient,
        Feature::Image,
        Feature::ImagePaint,
        Feature::ExtendNone,
        // Display P3 PNGs upload verbatim; the `Rgba16F` path carries the
        // linear primaries as well.
        Feature::ImageColorSpace(ImageColorSpace::DisplayP3),
        Feature::ImageColorSpace(ImageColorSpace::LinearP3),
        Feature::ImageColorSpace(ImageColorSpace::LinearSrgb),
        Feature::ImageF16,
        Feature::Backdrop,
        Feature::BackdropBlur,
        Feature::BackdropColorMatrix,
        Feature::BackdropEffect,
        Feature::BackdropScale,
        Feature::BackdropLevels,
        Feature::BackdropUnion,
        Feature::BackdropOuter,
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
///
/// `shader-paint` and `mesh-gradient` have no scene feature of their own;
/// they report the nearest declared one (`Fill`) while the `api` string
/// names the real construct.
fn unsupported_feature(u: &str) -> Feature {
    match u {
        "filter" => Feature::Filter,
        "path" | "path-clip-too-large" => Feature::Path,
        "sweep-gradient" => Feature::SweepGradient,
        "stroke-dash" => Feature::StrokeDash,
        "glyph-stroke" | "color-font" => Feature::Glyphs,
        "glyph-transform" => Feature::GlyphTransform,
        "shadow" => Feature::Shadow,
        "backdrop-unclipped"
        | "backdrop-footprint"
        | "backdrop-effect-sdf-path"
        | "backdrop-shader" => Feature::Backdrop,
        "blend-space" => Feature::BlendSpace(BlendSpace::SrgbEncoded),
        "projective-unclipped"
        | "projective-backdrop-member"
        | "projective-backdrop-cross-space" => Feature::Projective,
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

/// Applies a clip shape to a layer edit.
fn clip_shape(edit: &mut LayerEdit<Gpu>, shape: &ShapeKind) {
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
    engine: &GpuEngine<Gpu>,
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
    engine: &GpuEngine<Gpu>,
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
    engine: &GpuEngine<Gpu>,
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
                    ImageData::<Rgba16F>::new(width, height, blob.clone())
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
            ImageData::<Rgba8>::new(width, height, rgba)
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
    engine: &GpuEngine<Gpu>,
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
    engine: &GpuEngine<Gpu>,
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
    engine: &GpuEngine<Gpu>,
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
        LayerFilter::GaussianBlur { sigma } => {
            engine.filter(filters::GaussianBlur::new(*sigma as f32))
        }
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
    engine: &GpuEngine<Gpu>,
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
            .map(|shape| shape_kind(shape, cherenkov::FillRule::NonZero)),
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
        backdrop: layer.backdrop,
        backdrop_effect: layer
            .backdrop_effect
            .as_ref()
            .map(crate::convert::backdrop_effect)
            .transpose()?,
        backdrop_outer: crate::convert::backdrop_outer(
            layer.backdrop_outer,
            layer.backdrop.unwrap_or(0),
        )?,
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
    if !matches!(layer.items[index], Item::Draw(_)) {
        return Err(BenchError::Engine(
            "cherenkov: a live entry does not target a draw item".into(),
        ));
    }
    let kind = std::mem::discriminant(&match &layer.items[index] {
        Item::Draw(d) => op(d, fonts, images, blobs, &FRONT)?,
        Item::Layer(_) | Item::Group(_) => unreachable!("checked above"),
    });
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
    surface: &Surface<Gpu>,
    tx: &mut Transaction<'_, Gpu>,
    parent: Option<&GpuLayer>,
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
            let sample = prep.backdrop_effect.as_ref().map_or_else(
                || group.sample(),
                |effect| group.sample_with(effect.clone()),
            );
            edit.backdrop(sample.outer(prep.backdrop_outer));
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
    surface: &Surface<Gpu>,
    group: &cherenkov_scene::BackdropGroup,
) -> Result<cherenkov::BackdropGroup, BenchError> {
    use filtrate::filters::{ColorMatrix, GaussianBlur};
    let unsupported = || BenchError::Unsupported {
        engine: Cherenkov::NAME,
        feature: Feature::Backdrop,
        api: Some("backdrop filter chain shape is not built"),
    };
    let spec = convert::backdrop_spec(group)?;
    Ok(match group.filters.as_slice() {
        [] => surface.backdrop_group_unfiltered(spec),
        [BackdropFilter::GaussianBlur { sigma }] => {
            surface.backdrop_group(GaussianBlur::new(*sigma as f32), spec)
        }
        [BackdropFilter::ColorMatrix { matrix }] => {
            surface.backdrop_group(ColorMatrix(matrix.map(|v| v as f32)), spec)
        }
        [
            BackdropFilter::GaussianBlur { sigma },
            BackdropFilter::ColorMatrix { matrix },
        ] => surface.backdrop_group(
            GaussianBlur::new(*sigma as f32).then(ColorMatrix(matrix.map(|v| v as f32))),
            spec,
        ),
        _ => return Err(unsupported()),
    })
}

/// The engine, device and prepared scene state do not format.
impl std::fmt::Debug for Cherenkov {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cherenkov")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl Cherenkov {
    /// Adapter key.
    pub const NAME: &'static str = "cherenkov";

    /// Creates the adapter, initializing the GPU engine.
    ///
    /// # Errors
    /// [`BenchError::Gpu`] when no adapter exists or device creation fails.
    pub fn new() -> Result<Self, BenchError> {
        Self::make(None)
    }

    /// Creates the adapter with the #169 allocation diagnostic installed.
    ///
    /// # Errors
    /// [`BenchError::Gpu`] when no adapter exists or device creation fails.
    pub fn with_alloc_diag(sink: cherenkov_gpu::diag::Sink) -> Result<Self, BenchError> {
        Self::make(Some(sink))
    }

    /// A final snapshot event after the engine's last submission, so a
    /// trace ends with the post-drop allocator state.
    pub fn alloc_diag_teardown(&self) {
        if let Some(sink) = &self.alloc_diag {
            sink.teardown(&self.shared_device.device);
        }
    }

    fn make(alloc_diag: Option<cherenkov_gpu::diag::Sink>) -> Result<Self, BenchError> {
        let config = GpuConfig {
            timestamps: true,
            scratch_format: match std::env::var("CHERENKOV_SCRATCH_FORMAT").as_deref() {
                Ok("rgba8") => ScratchFormat::Rgba8Unorm,
                _ => ScratchFormat::LinearF16,
            },
            alloc_diag: alloc_diag.clone(),
            ..GpuConfig::default()
        };
        let shared_device = SharedDevice::create(&config)
            .map_err(|e| BenchError::Gpu(format!("cherenkov engine: {e}")))?;
        let engine = GpuEngine::<Gpu>::new(GpuConfig {
            device: Some(shared_device.clone()),
            ..config
        })
        .map_err(|e| BenchError::Gpu(format!("cherenkov engine: {e}")))?;
        Ok(Self {
            info: EngineInfo {
                name: Self::NAME,
                engine_crate: "cherenkov-gpu",
                crate_version: env!("DEP_CHERENKOV_GPU_VERSION"),
                source_rev: option_env!("DEP_CHERENKOV_GPU_SOURCE_REV").map(String::from),
                output_format: "wgpu Rgba16Float texture (premultiplied linear P3)".to_string(),
                precision: "analytic f32 quad instances; f16 working-space target",
                route: "wgpu (gpu slice)",
                color_note: "premultiplied linear Display P3 end to end; HDR channels unclamped",
                encode_scope: "records `cherenkov::Content` calls (fill/stroke/shadow/glyphs) \
                               against fonts and the layer tree prepared once",
            },
            engine,
            shared_device,
            surface: None,
            timings: Timings::default(),
            fonts: HashMap::new(),
            images: HashMap::new(),
            image_handles: Vec::new(),
            filter_handles: Vec::new(),
            content_layers: Vec::new(),
            backdrop_groups: HashMap::new(),
            has_motion: false,
            motion_committed: false,
            has_live: false,
            frame: 0,
            clock: Clock::new(),
            counters: Counters::default(),
            present: None,
            alloc_diag,
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

    fn present(&mut self, kind: PresentKind) -> Result<(), BenchError> {
        // The engine already runs on the bench-owned SharedDevice (#101), so
        // the presented destination can be read back on the same device and
        // queue.
        let delivery = shader_delivery(
            self.shared_device.adapter.get_info().backend,
            &self.shared_device.device,
        )
        .map_err(|e| BenchError::Engine(e.to_string()))?;
        self.present = Some(Box::new(Present {
            kind,
            presenter: Presenter::new(&self.shared_device.device, delivery),
            source: None,
            destination: None,
            headroom: 1.0,
        }));
        Ok(())
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "Display::headroom is f32 at the engine boundary"
    )]
    fn prepare(&mut self, input: &EncodeInput<'_>) -> Result<(), BenchError> {
        convert::check_features(Self::NAME, input.scene, &cherenkov_features(), |f| {
            convert::missing_api(&FRONT, f)
        })?;
        let size = (input.scene.width, input.scene.height);
        let surface = match self.present.as_deref_mut() {
            Some(present) => {
                let (target, textures) = TextureTarget::new(size);
                let surface = self
                    .engine
                    .surface(target, || {})
                    .map_err(|e| BenchError::Gpu(format!("cherenkov surface: {e}")))?;
                present.source =
                    Some(textures.try_recv().map_err(|e| {
                        BenchError::Engine(format!("cherenkov texture target: {e}"))
                    })?);
                present.destination = Some(present_target(
                    &self.shared_device.device,
                    size,
                    present.kind,
                ));
                surface
            }
            None => self
                .engine
                .surface(Offscreen::new(size, OffscreenFormat::LinearF16), || {})
                .map_err(|e| BenchError::Gpu(format!("cherenkov surface: {e}")))?,
        };
        if let Some(present) = self.present.as_deref_mut() {
            // Announce the scene's declared headroom as a host display
            // would; the present pass reads it every frame (#97).
            present.headroom = input.scene.present_headroom as f32;
            let _ = surface.display(cherenkov::Display {
                headroom: input.scene.present_headroom as f32,
                ..cherenkov::Display::default()
            });
        }
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
        let stats = self.engine.stats();
        let render_seconds = render_at.elapsed().as_secs_f64();
        let readback_at = std::time::Instant::now();
        let image = if !readback {
            None
        } else if let Some(present) = self.present.as_deref_mut() {
            // The presented destination, lifted back into the working
            // space for the sRGB kinds.
            Some(present.read(&self.shared_device)?)
        } else {
            let rb = self
                .surface
                .as_ref()
                .ok_or_else(|| BenchError::Engine("cherenkov: submit before prepare".into()))?
                .readback()
                .map_err(render_error)?;
            Some(cherenkov_oracle::F32Image {
                width: rb.width,
                height: rb.height,
                pixels: rb.pixels,
            })
        };
        let phases = stats.phases;
        Ok(Submit {
            image,
            gpu: Vec::new(),
            phases: Some(crate::Phases {
                lower: phases.lower_seconds,
                encode: phases.encode_seconds,
                stamp: phases.stamp_seconds,
                wait: phases.wait_seconds,
            }),
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
            adapter: Some(info.name.clone()),
            backend: Some(info.backend.clone()),
            driver: Some(info.driver.clone()),
            driver_info: Some(info.driver_info.clone()),
            vendor: Some(info.vendor),
            device: Some(info.device),
            target_format: Some("Rgba16Float".to_string()),
            cpu: crate::cpu_model(),
            thermal_celsius: crate::thermal_celsius(),
        }
    }

    fn memory(&self) -> AdapterMemory {
        let usage = self.engine.memory();
        let adapter_info = self.shared_device.adapter.get_info();
        AdapterMemory {
            engine: Reading::Measured(EngineBytes {
                cpu_bytes: usage.cpu.0,
                gpu_bytes: usage.gpu.0,
                backdrop_capture_bytes: usage.backdrop_captures.0,
            }),
            wgpu_allocator: wgpu_allocator(&self.shared_device.device, adapter_info.backend),
            skia_budgeted: Reading::unavailable("not a Skia adapter"),
            vk_memory_budget: wgpu_vk_memory_budget(
                &self.shared_device.device,
                adapter_info.backend,
                &adapter_info.name,
            ),
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
            ShapeKind::Path { data, .. } => c.shadow(Fixed(data.clone()), Fixed(*shadow)),
        },
        Op::Glyphs { run, paint } => c.glyphs(Fixed(run.clone()), Fixed(paint.clone())),
        Op::Image {
            image,
            dst,
            sampling,
        } => c.image(*image, Fixed(*dst), *sampling),
        Op::Group { group, ops } => c.group(Fixed(*group), |c| {
            for op in ops {
                record_op(c, op);
            }
        }),
        Op::Text { layout, origin } => cherenkov::draw_text(c, layout, *origin),
    }
}
