//! Stage-1 lowering: a display list becomes a device-independent op
//! stream, patchable in place along [`cherenkov::Dirty`] ranges.

use std::sync::Arc;

use rustc_hash::FxHashMap;

use cherenkov::kurbo::{Affine, Line, PathEl, Rect};
use cherenkov::lowering::rounded_box::{BoxForm, box_form};
use cherenkov::{
    BlendMode, BlendSpace, Command, Extend, FillRule, ImageId, ImagePattern, Interpolation, Paint,
    RenderError, ShapeData,
};
use cherenkov::{GlyphRun, GlyphStyle};
use filtrate_core::space::{P3_TO_SRGB, srgb_encode, transform};

use super::instance::{
    EXTEND_NONE, EXTEND_PAD, EXTEND_REFLECT, EXTEND_REPEAT, FLAG_HAS_INNER, INTERP_SRGB,
    INTERP_WORKING, KIND_FILL, KIND_STROKE_DIST, KIND_STROKE_OFFSET, PAINT_IMAGE, PAINT_LINEAR,
    PAINT_MESH, PAINT_RADIAL, PAINT_SOLID, PAINT_SWEEP, Shape, Stop,
};
use super::path;
use crate::names;
use crate::render::GpuImage;
use crate::render::glyph::FontData;
use skrifa::MetadataProvider as _;
use skrifa::raw::TableProvider as _;
use skrifa::raw::types::F2Dot14;

/// f64 to f32; instance data is f32 by design.
#[expect(clippy::cast_possible_truncation)]
const fn f32_f64(v: f64) -> f32 {
    v as f32
}

/// A `ShapeData` expressed as a centred rounded box.
pub struct Boxed {
    /// Extra local transform (centre translation, plus rotation for
    /// ellipses and stroked lines).
    pub extra: Affine,
    /// The centred shape.
    pub shape: Shape,
    /// The local bounds, centred.
    pub bounds: Rect,
}

/// Converts a semantic shape into a centred rounded box plus the local
/// transform that centres it: the shared [`box_form`] mapping, packed as
/// instance data.
///
/// A shape without area draws nothing, so it returns `None`.
pub fn box_shape(shape: &ShapeData) -> Result<Option<Boxed>, RenderError> {
    match box_form(shape) {
        BoxForm::Box { extra, shape } => Ok(Some(Boxed {
            extra,
            shape: shape.into(),
            bounds: rect_around_origin(shape.half),
        })),
        BoxForm::Empty => Ok(None),
        BoxForm::Path => Err(RenderError::Unsupported(names::PATH)),
    }
}

fn rect_around_origin(half: [f32; 2]) -> Rect {
    Rect::new(
        -f64::from(half[0]),
        -f64::from(half[1]),
        f64::from(half[0]),
        f64::from(half[1]),
    )
}

/// The blur sigma the shader integrates against, modelling the oracle's
/// pixel-area sampling: `sqrt(sigma² + 1/12)` for a positive sigma.
fn shadow_sigma(sigma: f64) -> f64 {
    if sigma > 0.0 {
        sigma.mul_add(sigma, 1.0 / 12.0).sqrt()
    } else {
        sigma
    }
}

/// Exact `2.0`/`1.0` comparisons: these fields only ever hold the constants
/// [`box_shape`] assigns.
#[expect(clippy::float_cmp)]
fn shape_is_offsettable(shape: Shape) -> bool {
    shape.exponent == 2.0 && shape.aspect == 1.0
}

/// Whether the analytic stroke kernels reproduce `stroke`'s joins on `shape`.
///
/// A corner with a positive radius is a smooth curve: there is no join, so
/// every join style and miter limit draw the same outline. A sharp corner of
/// an offsettable (circular-corner, aspect-1) shape is a right angle whose
/// miter ratio is 1/sin(45°) = √2: a round join is the offset shape's
/// quarter-circle, and a miter join with limit ≥ √2 keeps the sharp corner.
/// Below √2 kurbo bevels the corner, and a bevel join always does, neither of
/// which the offset kernel can express. A sharp corner of a non-offsettable
/// shape (superelliptical, or elliptical with aspect ≠ 1) is drawn by the
/// distance kernel, which only produces the round join.
fn analytic_join(shape: Shape, stroke: &kurbo::Stroke) -> bool {
    let sharp = shape.radii.iter().any(|&r| r <= 0.0);
    if !sharp {
        return true;
    }
    match stroke.join {
        kurbo::Join::Round => true,
        kurbo::Join::Miter => {
            shape_is_offsettable(shape) && stroke.miter_limit >= std::f64::consts::SQRT_2
        }
        kurbo::Join::Bevel => false,
    }
}

/// The path cache tag for a fill rule.
const fn fill_tag(rule: FillRule) -> u64 {
    match rule {
        FillRule::NonZero => 0,
        FillRule::EvenOdd => 1,
    }
}

/// The paint data shared by every instance kind.
#[derive(Clone, Copy, Default)]
pub struct PaintData {
    /// `PAINT_*`.
    pub kind: u32,
    /// Solid colour.
    pub color: [f32; 4],
    /// Gradient/image coefficients (see `Instance::grad`).
    pub grad: [f32; 4],
    /// Gradient/image coefficients (see `Instance::grad2`).
    pub grad2: [f32; 4],
    /// First stop, relative to the resolved paint's stop buffer.
    pub first_stop: u32,
    /// `count | interp << 16 | extend << 20`; for `PAINT_IMAGE`,
    /// `extend_x | extend_y << 4 | sampling << 8 | image.sampling_flags`.
    pub packed: u32,
    /// A registered image ID. Shader identities belong to the emitted range,
    /// keeping ordinary paint fields trivially copyable.
    pub image: Option<u64>,
}

/// Resolved solid paint stays inline. Gradient and image fields are only
/// allocated for paints that use them, keeping every prepared op compact.
#[derive(Clone)]
pub enum ResolvedPaint {
    /// Working-space solid color.
    Solid([f32; 4]),
    /// Gradient/image shader fields and their stop buffer.
    Resources(Box<(PaintData, Vec<Stop>)>),
    /// Deferred until device scale and full shape bounds are known.
    Shader(Box<ResolvedShader>),
}

/// A shader use and the independent shape-local sampling transform.
#[derive(Clone)]
pub struct ResolvedShader {
    pub source: cherenkov::ShaderPaint,
    pub sampling: Affine,
}

fn push_stops(
    stops: &mut Vec<Stop>,
    gradient_stops: &[cherenkov::ColorStop],
    interpolation: Interpolation,
    extend: Extend,
) -> (u32, u32) {
    let first = u32::try_from(stops.len()).unwrap_or(u32::MAX);
    let mut sorted = gradient_stops.to_vec();
    sorted.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    let count = u32::try_from(sorted.len().min(0xffff)).unwrap_or(0xffff);
    for stop in sorted.iter().take(count as usize) {
        let [r, g, b, a] = stop.color.components;
        let color = if interpolation == Interpolation::SrgbEncoded {
            // The CPU backend's stop conversion (`cpu/src/render/paint.rs`).
            let [sr, sg, sb] = transform(&P3_TO_SRGB, [r, g, b]).map(srgb_encode);
            [sr, sg, sb, a]
        } else {
            [r, g, b, a]
        };
        stops.push(Stop {
            color,
            offset: stop.offset,
            pad: [0.0; 3],
        });
    }
    let interp = match interpolation {
        Interpolation::Working => INTERP_WORKING,
        Interpolation::SrgbEncoded => INTERP_SRGB,
    };
    (first, count | (interp << 16) | (extend_code(extend) << 20))
}

const fn extend_code(extend: Extend) -> u32 {
    match extend {
        Extend::Pad => EXTEND_PAD,
        Extend::Repeat => EXTEND_REPEAT,
        Extend::Reflect => EXTEND_REFLECT,
        Extend::None => EXTEND_NONE,
    }
}

/// The image space -> content transform that maps a `width` x `height`
/// image onto the whole of `dst`.
pub(super) fn image_placement(dst: &Rect, width: u32, height: u32) -> Affine {
    Affine::translate((dst.x0, dst.y0))
        * Affine::scale_non_uniform(
            (dst.x1 - dst.x0) / f64::from(width),
            (dst.y1 - dst.y0) / f64::from(height),
        )
}

/// An image paint over a texture of `width` x `height` with source
/// conversion `sampling_flags`; `transform` maps image space to content
/// space and `to_local` content space to the instance's local space.
#[expect(
    clippy::cast_precision_loss,
    clippy::many_single_char_names,
    reason = "image dimensions fit f32; affine coefficients are conventionally a..f"
)]
pub(super) fn image_paint(
    to_local: Affine,
    transform: Affine,
    (width, height): (u32, u32),
    (extend_x, extend_y): (Extend, Extend),
    sampling: cherenkov::Sampling,
    sampling_flags: u32,
) -> PaintData {
    // `to_local * transform` maps image space to instance-local; the
    // shader needs the inverse.
    let [a, b, c, d, e, f] = (to_local * transform).inverse().as_coeffs();
    let sampling = match sampling {
        cherenkov::Sampling::Nearest => 0,
        cherenkov::Sampling::Linear => 1,
    };
    PaintData {
        kind: PAINT_IMAGE,
        grad: [f32_f64(a), f32_f64(b), f32_f64(c), f32_f64(d)],
        grad2: [f32_f64(e), f32_f64(f), width as f32, height as f32],
        packed: extend_code(extend_x)
            | (extend_code(extend_y) << 4)
            | (sampling << 8)
            | sampling_flags,
        ..PaintData::default()
    }
}

/// Lowers a paint; `to_local` maps content space to the instance's local
/// (shape-centred) space in which the shader evaluates gradient parameters.
fn paint_data(
    paint: &Paint,
    to_local: Affine,
    stops: &mut Vec<Stop>,
    images: &FxHashMap<u64, GpuImage>,
) -> Result<PaintData, RenderError> {
    let mut data = PaintData {
        kind: PAINT_SOLID,
        ..PaintData::default()
    };
    match paint {
        Paint::Solid(c) => data.color = c.components,
        Paint::Linear(g) => {
            data.kind = PAINT_LINEAR;
            let start = to_local * g.start;
            let end = to_local * g.end;
            data.grad = [
                f32_f64(start.x),
                f32_f64(start.y),
                f32_f64(end.x),
                f32_f64(end.y),
            ];
            let (first, packed) = push_stops(stops, &g.stops, g.interpolation, g.extend);
            data.first_stop = first;
            data.packed = packed;
        }
        Paint::Radial(g) => {
            let identical = g.has_identical_circles();
            if identical && matches!(g.extend, Extend::Repeat | Extend::Reflect) {
                return Err(RenderError::IdenticalRadialCircles(g.extend));
            }
            data.kind = PAINT_RADIAL;
            let c0 = to_local * g.start_center;
            let c1 = to_local * g.end_center;
            data.grad = [f32_f64(c0.x), f32_f64(c0.y), f32_f64(c1.x), f32_f64(c1.y)];
            data.grad2 = [
                f32_f64(g.start_radius),
                f32_f64(g.end_radius),
                if identical { 1.0 } else { 0.0 },
                0.0,
            ];
            let (first, packed) = push_stops(stops, &g.stops, g.interpolation, g.extend);
            data.first_stop = first;
            data.packed = packed;
        }
        Paint::Sweep(g) => {
            data.kind = PAINT_SWEEP;
            let center = to_local * g.center;
            data.grad = [f32_f64(center.x), f32_f64(center.y), 0.0, 0.0];
            data.grad2 = [f32_f64(g.start_angle), f32_f64(g.end_angle), 0.0, 0.0];
            let (first, packed) = push_stops(stops, &g.stops, g.interpolation, g.extend);
            data.first_stop = first;
            data.packed = packed;
        }
        Paint::Mesh(mesh) => return mesh_paint(mesh, to_local, stops),
        Paint::Image(pattern) => {
            let img = images.get(&pattern.image.raw()).ok_or_else(|| {
                RenderError::Image(format!("unregistered image {}", pattern.image.raw()))
            })?;
            data = PaintData {
                image: Some(pattern.image.raw()),
                ..image_paint(
                    to_local,
                    pattern.transform,
                    (img.width, img.height),
                    (pattern.extend_x, pattern.extend_y),
                    pattern.sampling,
                    img.sampling_flags,
                )
            };
        }
        Paint::Shader(_) => return Err(RenderError::Unsupported(names::SHADER)),
        Paint::Transformed(_) => unreachable!("transformed paints resolve separately"),
    }
    Ok(data)
}

/// Pack mesh-only resources without growing ordinary paint preparation.
fn mesh_paint(
    mesh: &cherenkov::MeshGradient,
    to_local: Affine,
    stops: &mut Vec<Stop>,
) -> Result<PaintData, RenderError> {
    let mut data = PaintData {
        kind: PAINT_MESH
            | if mesh.interpolation_mode() == cherenkov::MeshColorInterpolation::Smoothstep {
                super::instance::PAINT_MESH_SMOOTH
            } else {
                0
            },
        ..PaintData::default()
    };
    data.first_stop = u32::try_from(stops.len())
        .map_err(|_| RenderError::Render("mesh paint buffer exceeds u32".into()))?;
    data.packed = mesh
        .columns()
        .checked_mul(mesh.rows())
        .filter(|count| {
            *count <= 0x00ff_ffff
                && count
                    .checked_mul(6)
                    .and_then(|len| len.checked_add(data.first_stop))
                    .is_some()
        })
        .ok_or_else(|| RenderError::Render("mesh paint buffer exceeds u32".into()))?;
    let stride = mesh.columns() as usize + 1;
    for row in 0..mesh.rows() as usize {
        for column in 0..mesh.columns() as usize {
            let base = row * stride + column;
            let indices = [base, base + 1, base + stride, base + stride + 1];
            let points = indices.map(|index| to_local * mesh.points()[index]);
            if points
                .iter()
                .any(|p| !p.is_finite() || !f32_f64(p.x).is_finite() || !f32_f64(p.y).is_finite())
            {
                return Err(RenderError::Render(
                    "mesh points must be finite GPU coordinates".into(),
                ));
            }
            for pair in points.as_chunks::<2>().0 {
                stops.push(Stop {
                    color: [
                        f32_f64(pair[0].x),
                        f32_f64(pair[0].y),
                        f32_f64(pair[1].x),
                        f32_f64(pair[1].y),
                    ],
                    offset: 0.0,
                    pad: [0.0; 3],
                });
            }
            for index in indices {
                let [red, green, blue, alpha] = mesh.colors()[index].components;
                stops.push(Stop {
                    color: [red * alpha, green * alpha, blue * alpha, alpha],
                    offset: 0.0,
                    pad: [0.0; 3],
                });
            }
        }
    }
    Ok(data)
}

/// Resolves `paint` device-independently; `to_local` is the
/// instance-local transform the shader paints in (the boxed `extra`
/// inverse, or identity for device-space replay).
#[inline]
fn resolve(
    paint: &Paint,
    to_local: Affine,
    images: &FxHashMap<u64, GpuImage>,
) -> Result<ResolvedPaint, RenderError> {
    if let Paint::Solid(color) = paint {
        return Ok(ResolvedPaint::Solid(color.components));
    }
    resolve_resources(paint, to_local, images)
}

/// Keep allocation and shader ownership out of solid paint preparation.
#[inline(never)]
fn resolve_resources(
    paint: &Paint,
    to_local: Affine,
    images: &FxHashMap<u64, GpuImage>,
) -> Result<ResolvedPaint, RenderError> {
    if let Paint::Transformed(_) = paint {
        return resolve_transformed(paint, to_local, images);
    }
    if let Paint::Shader(shader) = paint {
        return Ok(ResolvedPaint::Shader(Box::new(ResolvedShader {
            source: shader.clone(),
            sampling: Affine::IDENTITY,
        })));
    }
    let mut stops = Vec::new();
    let data = paint_data(paint, to_local, &mut stops, images)?;
    Ok(ResolvedPaint::Resources(Box::new((data, stops))))
}

/// Resolves only explicit paint transforms, preserving the ordinary paint
/// preparation path and instance size.
fn resolve_transformed(
    mut paint: &Paint,
    to_local: Affine,
    images: &FxHashMap<u64, GpuImage>,
) -> Result<ResolvedPaint, RenderError> {
    let mut transform = Affine::IDENTITY;
    while let Paint::Transformed(mapped) = paint {
        if !mapped.transform.is_finite() || !mapped.transform.inverse().is_finite() {
            return Err(RenderError::Render(
                "paint transform must be finite and invertible".into(),
            ));
        }
        transform *= mapped.transform;
        paint = &mapped.paint;
    }
    if transform == Affine::IDENTITY {
        return resolve(paint, to_local, images);
    }
    let inverse = (to_local * transform).inverse();
    if !inverse.is_finite() {
        return Err(RenderError::Render(
            "composed paint transform must be finite and invertible".into(),
        ));
    }
    if let Paint::Shader(shader) = paint {
        return Ok(ResolvedPaint::Shader(Box::new(ResolvedShader {
            source: shader.clone(),
            sampling: to_local * transform.inverse() * to_local.inverse(),
        })));
    }
    if let Paint::Solid(color) = paint {
        return Ok(ResolvedPaint::Solid(color.components));
    }
    if let Paint::Image(pattern) = paint {
        // Images already carry a full sampling affine. Compose in f64 once,
        // before rounding to GPU coefficients; two f32 maps can disagree at
        // nearest-neighbour texel boundaries.
        let mut pattern = pattern.clone();
        pattern.transform = transform * pattern.transform;
        let mut stops = Vec::new();
        let data = paint_data(&Paint::Image(pattern), to_local, &mut stops, images)?;
        return Ok(ResolvedPaint::Resources(Box::new((data, stops))));
    }
    let [xx, yx, xy, yy, tx, ty] = inverse.as_coeffs();
    let mut stops = vec![
        Stop {
            color: [f32_f64(xx), f32_f64(yx), f32_f64(xy), f32_f64(yy)],
            offset: 0.0,
            pad: [0.0; 3],
        },
        Stop {
            color: [f32_f64(tx), f32_f64(ty), 0.0, 0.0],
            offset: 0.0,
            pad: [0.0; 3],
        },
    ];
    let mut data = paint_data(paint, Affine::IDENTITY, &mut stops, images)?;
    // Images have no gradient stops, but still point just past the header.
    data.first_stop = 2;
    data.kind |= super::instance::PAINT_TRANSFORMED;
    Ok(ResolvedPaint::Resources(Box::new((data, stops))))
}

/// A clip shape, resolved at lowering.
pub enum ClipShape {
    /// A shape with no area (`Line`): the scope draws nothing.
    Empty,
    /// A centred rounded box clip.
    Boxed {
        /// The boxed shape's local transform.
        extra: Affine,
        /// The centred clip shape.
        shape: Shape,
        /// The scene `Rect`, when the source shape was one (the compose
        /// stage computes the device-space candidate).
        rect: Option<Rect>,
    },
    /// A path clip rasterized into a coverage mask at compose.
    Path {
        /// The path elements.
        elements: Arc<[PathEl]>,
        /// The fill rule.
        rule: FillRule,
    },
}

/// The outline a `Path` op rasterizes on a cache miss.
pub enum Outline {
    /// Geometry borrowed from this content's retained source list.
    Source {
        /// Root command index.
        command: usize,
        /// A fill's hash; strokes include tolerance at realization.
        content: Option<u64>,
    },
    /// A fill's local geometry and its exact content identity.
    Fill {
        /// The path elements.
        elements: Arc<[PathEl]>,
        /// Hash of the elements and fill rule.
        content: u64,
    },
    /// Stroke `shape` with `stroke`; the device-dependent tolerance is
    /// mixed into the content hash at compose.
    Stroke {
        /// The stroked shape.
        shape: ShapeData,
        /// The stroke style.
        stroke: kurbo::Stroke,
    },
}

/// Borrowed geometry after resolving a retained command reference.
#[derive(Clone, Copy)]
pub enum OutlineRef<'a> {
    /// Fill geometry and its precomputed semantic hash.
    Fill {
        elements: &'a [PathEl],
        content: u64,
    },
    /// Stroke geometry whose hash also depends on device tolerance.
    Stroke {
        shape: &'a ShapeData,
        stroke: &'a kurbo::Stroke,
    },
}

impl Outline {
    /// Resolve owned and source-backed outlines without constructing geometry.
    #[expect(
        clippy::inline_always,
        reason = "resolve borrowed outline fields at their use site"
    )]
    #[inline(always)]
    pub fn get<'a>(&'a self, source: &'a cherenkov::DisplayList) -> OutlineRef<'a> {
        match self {
            Self::Fill { elements, content } => OutlineRef::Fill {
                elements,
                content: *content,
            },
            Self::Stroke { shape, stroke } => OutlineRef::Stroke { shape, stroke },
            Self::Source { command, content } => match &source.commands()[*command] {
                Command::Fill {
                    shape: ShapeData::Path { elements, .. },
                    ..
                } => OutlineRef::Fill {
                    elements,
                    content: content.expect("prepared fill has a hash"),
                },
                Command::Stroke { shape, stroke, .. } => OutlineRef::Stroke { shape, stroke },
                _ => unreachable!("prepared outline keeps its source kind"),
            },
        }
    }
}

impl OutlineRef<'_> {
    pub fn gpu_candidate(self, complexity: path::Complexity, raster: Affine) -> bool {
        match self {
            Self::Fill { .. } => complexity.fill_candidate(),
            Self::Stroke { stroke, .. } => complexity.stroke_candidate(stroke, raster),
        }
    }

    /// Semantic identity at the tolerance used to construct the outline.
    pub fn content(self, tolerance: f64) -> u64 {
        match self {
            Self::Fill { content, .. } => content,
            Self::Stroke { shape, stroke } => path::hash_stroke(shape, stroke, tolerance),
        }
    }

    /// Construct local geometry only when a realization needs it.
    pub fn build(self, tolerance: f64) -> kurbo::BezPath {
        match self {
            Self::Fill { elements, .. } => kurbo::BezPath::from_vec(elements.to_vec()),
            Self::Stroke { shape, stroke } => {
                cherenkov_stroke::stroke(path::shape_path(shape, tolerance), stroke, tolerance)
            }
        }
    }
}

/// A prepared glyph run either indexes retained source or owns expanded glyphs.
pub enum GlyphSource {
    /// A partial run produced by nested pictures or COLR expansion.
    Run(GlyphRun),
    /// An unchanged root run; count preserves structural patch validation.
    Command { index: usize, count: usize },
}

impl GlyphSource {
    fn len(&self) -> usize {
        match self {
            Self::Run(run) => run.glyphs.len(),
            Self::Command { count, .. } => *count,
        }
    }

    pub fn get<'a>(&'a self, source: &'a cherenkov::DisplayList) -> &'a GlyphRun {
        match self {
            Self::Run(run) => run,
            Self::Command { index, .. } => {
                let Command::Glyphs { run, .. } = &source.commands()[*index] else {
                    unreachable!("prepared glyph command keeps its source kind");
                };
                run
            }
        }
    }
}

/// A stage-1 draw or scope op: device-independent.
pub enum Op {
    /// A shaped SDF instance (fill/stroke box paths, stroked lines, image
    /// draws).
    Shaped {
        /// `KIND_*`.
        kind: u32,
        /// Content→instance-local: the accumulated ambient transform
        /// (BeginTransform/Picture/COLR placements) times `boxed.extra`.
        local: Affine,
        /// The ambient transform without `boxed.extra` (for the AA
        /// margin at compose).
        ambient: Affine,
        /// The centred shape.
        shape: Shape,
        /// The inner shape of an offset stroke.
        inner: Option<Shape>,
        /// Uninflated local bounds.
        bounds: Rect,
        /// The margin that is not antialiasing (half width for strokes).
        extra_margin: f64,
        /// The resolved paint.
        paint: ResolvedPaint,
        /// `params[0]` (stroke half width).
        param_x: f32,
        /// `meta[3]` flag bits (e.g. `FLAG_HAS_INNER`).
        flags: u32,
    },
    /// A Gaussian-blurred box.
    Shadow {
        /// Content→instance-local (ambient × offset × `boxed.extra`).
        local: Affine,
        /// The ambient transform (AA margin).
        ambient: Affine,
        /// The centred shape.
        shape: Shape,
        /// Uninflated local bounds.
        bounds: Rect,
        /// `sqrt(sigma² + 1/12)`.
        sigma_eff: f64,
        /// The shadow colour.
        color: [f32; 4],
    },
    /// A path fill or stroked outline; the rasterizer runs at compose on
    /// an atlas miss.
    Path {
        /// The ambient transform.
        local: Affine,
        /// The fill rule.
        rule: FillRule,
        /// Source complexity used before device-space preparation.
        complexity: path::Complexity,
        /// The outline builder.
        outline: Outline,
        /// The resolved paint (identity local space — device pixels).
        paint: ResolvedPaint,
    },
    /// A glyph run with the COLR glyphs already expanded into their
    /// picture ops at their placements.
    Glyphs {
        /// The ambient transform.
        local: Affine,
        /// The run, minus its COLR glyphs.
        run: GlyphSource,
        /// Conservative local outline bounds, independent of device placement.
        bounds: Rect,
        /// The resolved paint (identity local space).
        paint: ResolvedPaint,
    },
    /// A size-independent bitmap glyph realized at device scale.
    BitmapGlyph {
        /// The content transform at the glyph's origin.
        local: Affine,
        /// The registered font identity.
        font: u64,
        /// The glyph index.
        glyph: u32,
        /// The run size.
        size: f32,
    },
    /// Open a clip scope.
    BeginClip {
        /// The ambient transform (the clip's own `extra` is inside
        /// [`ClipShape`]).
        local: Affine,
        /// The clip shape.
        shape: ClipShape,
        /// Op index of the matching `End` op.
        end: u32,
    },
    /// Open an isolation scope (`BeginGroup` with opacity < 1, a
    /// non-normal blend or a non-linear blend space).
    BeginIsolate {
        /// The group opacity.
        opacity: f32,
        /// The group blend mode.
        blend: BlendMode,
        /// The group's declared compositing space: members composite
        /// with each other in it, then the group blends onto the
        /// backdrop in it.
        space: BlendSpace,
        /// Filter over the captured group.
        filter: Option<cherenkov::FilterId>,
        /// Op index of the matching `End` op.
        end: u32,
    },
    /// A captured silhouette with local blur and morphology parameters.
    BeginShadow {
        /// Content-space parameters, composed with sampled layer placement.
        parameters: super::shadow::Parameters,
        /// End of the generated silhouette draw.
        end: u32,
    },
    /// Close the innermost clip or isolate scope.
    End,
}

impl cherenkov::lowering::Operation for Op {
    fn end_mut(&mut self) -> Option<&mut u32> {
        match self {
            Self::BeginClip { end, .. }
            | Self::BeginIsolate { end, .. }
            | Self::BeginShadow { end, .. } => Some(end),
            _ => None,
        }
    }
    fn same_structure(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::BeginClip { shape: a, .. }, Self::BeginClip { shape: b, .. })
                if std::mem::discriminant(a) != std::mem::discriminant(b) =>
            {
                return false;
            }
            (
                Self::BeginIsolate {
                    blend: a,
                    space: sa,
                    ..
                },
                Self::BeginIsolate {
                    blend: b,
                    space: sb,
                    ..
                },
            ) if a != b || sa != sb => {
                return false;
            }
            _ => {}
        }
        std::mem::discriminant(self) == std::mem::discriminant(other)
            && !matches!((self, other), (Self::Glyphs { run: a, .. }, Self::Glyphs { run: b, .. }) if a.len() != b.len())
    }
}

/// Resources needed while resolving content-space operations.
pub struct Lowerer<'a> {
    /// COLR cache writes committed after parallel preparation.
    pub pending: &'a mut Vec<super::glyph::PendingRaster>,
    /// Fonts for colour glyph expansion.
    pub fonts: &'a FxHashMap<u64, FontData>,
    /// Images for resolving image paint dimensions.
    pub images: &'a FxHashMap<u64, GpuImage>,
}

impl cherenkov::lowering::Compiler for Lowerer<'_> {
    type Op = Op;
    type Error = RenderError;
    fn draw(
        &mut self,
        command: &Command,
        ambient: Affine,
        ops: &mut Vec<Op>,
    ) -> Result<(), RenderError> {
        self.draw_at(command, ambient, ops, None)
    }

    #[inline]
    fn draw_at(
        &mut self,
        command: &Command,
        ambient: Affine,
        ops: &mut Vec<Op>,
        source: Option<usize>,
    ) -> Result<(), RenderError> {
        match command {
            Command::Fill { shape, paint } => self.fill(ambient, shape, paint, ops, source),
            Command::Stroke {
                shape,
                stroke,
                paint,
            } => self.stroke(ambient, shape, stroke, paint, ops, source),
            Command::Shadow { shape, shadow } => self.shadow(ambient, shape, shadow, ops),
            Command::Glyphs { run, paint } => self.glyph_run(ambient, run, paint, ops, source),
            Command::Image {
                image,
                dst,
                sampling,
            } => self.image_draw(ambient, *image, dst, *sampling, ops),
            _ => unreachable!("shared walker handles scopes and pictures"),
        }
    }
    fn clip(&mut self, shape: &ShapeData, ambient: Affine) -> Result<Op, RenderError> {
        Ok(Op::BeginClip {
            local: ambient,
            shape: clip_shape(shape)?,
            end: 0,
        })
    }
    fn group(
        &mut self,
        group: &cherenkov::Group,
        isolate: bool,
    ) -> Result<Option<Op>, RenderError> {
        Ok((isolate
            || group.filter.is_some()
            || group.opacity < 1.0
            || group.blend != BlendMode::Normal
            || group.blend_space != BlendSpace::Linear)
            .then_some(Op::BeginIsolate {
                opacity: group.opacity,
                blend: group.blend,
                space: group.blend_space,
                filter: group.filter,
                end: 0,
            }))
    }
    fn end(&mut self) -> Op {
        Op::End
    }
}

impl Lowerer<'_> {
    /// `Fill`: a shaped quad; a path goes through the coverage
    /// rasterizer.
    #[inline]
    fn fill(
        &self,
        ambient: Affine,
        shape: &ShapeData,
        paint: &Paint,
        ops: &mut Vec<Op>,
        source: Option<usize>,
    ) -> Result<(), RenderError> {
        if let ShapeData::Path { elements, rule } = shape {
            ops.push(Op::Path {
                local: ambient,
                rule: *rule,
                complexity: path::Complexity::of(elements),
                outline: source.map_or_else(
                    || Outline::Fill {
                        elements: Arc::clone(elements),
                        content: path::hash_elements(elements, fill_tag(*rule)),
                    },
                    |command| Outline::Source {
                        command,
                        content: Some(path::hash_elements(elements, fill_tag(*rule))),
                    },
                ),
                paint: resolve(paint, Affine::IDENTITY, self.images)?,
            });
            return Ok(());
        }
        let Some(boxed) = box_shape(shape)? else {
            return Ok(());
        };
        ops.push(Op::Shaped {
            kind: KIND_FILL,
            local: ambient * boxed.extra,
            ambient,
            shape: boxed.shape,
            inner: None,
            bounds: boxed.bounds,
            extra_margin: 0.0,
            paint: resolve(paint, boxed.extra.inverse(), self.images)?,
            param_x: 0.0,
            flags: 0,
        });
        Ok(())
    }

    /// `Image`: a fill of `dst` whose paint maps the rect onto the whole
    /// image, pad-extended.
    fn image_draw(
        &self,
        ambient: Affine,
        image: ImageId,
        dst: &Rect,
        sampling: cherenkov::Sampling,
        ops: &mut Vec<Op>,
    ) -> Result<(), RenderError> {
        let img = self
            .images
            .get(&image.raw())
            .ok_or_else(|| RenderError::Image(format!("unregistered image {}", image.raw())))?;
        if dst.x1 - dst.x0 <= 0.0 || dst.y1 - dst.y0 <= 0.0 {
            return Ok(());
        }
        let transform = image_placement(dst, img.width, img.height);
        let paint = Paint::Image(ImagePattern {
            image,
            transform,
            extend_x: Extend::Pad,
            extend_y: Extend::Pad,
            sampling,
        });
        self.fill(ambient, &ShapeData::Rect(*dst), &paint, ops, None)
    }

    /// `Stroke`: offset strokes for circular-corner boxes, distance
    /// strokes for continuous corners and ellipses, a box fast path for
    /// lines; paths and dashed strokes become `Outline::Stroke`.
    #[inline]
    fn stroke(
        &self,
        ambient: Affine,
        shape: &ShapeData,
        stroke: &kurbo::Stroke,
        paint: &Paint,
        ops: &mut Vec<Op>,
        source: Option<usize>,
    ) -> Result<(), RenderError> {
        cherenkov::lowering::check_dashes(stroke)?;
        let hw = stroke.width / 2.0;
        let general = match shape {
            ShapeData::Path { .. } => true,
            _ if !stroke.dash_pattern.is_empty() => true,
            ShapeData::Line(_) => stroke.start_cap != stroke.end_cap,
            _ => match box_shape(shape)? {
                None => return Ok(()),
                Some(boxed) => !analytic_join(boxed.shape, stroke),
            },
        };
        if general {
            ops.push(Op::Path {
                local: ambient,
                rule: FillRule::NonZero,
                complexity: path::Complexity::of_shape(shape),
                outline: source.map_or_else(
                    || Outline::Stroke {
                        shape: shape.clone(),
                        stroke: stroke.clone(),
                    },
                    |command| Outline::Source {
                        command,
                        content: None,
                    },
                ),
                paint: resolve(paint, Affine::IDENTITY, self.images)?,
            });
            return Ok(());
        }
        if let ShapeData::Line(line) = shape {
            return self.stroke_line(ambient, line, hw, stroke, paint, ops);
        }
        let Some(boxed) = box_shape(shape)? else {
            return Ok(());
        };
        if shape_is_offsettable(boxed.shape) {
            // Offset stroke: outer minus inner.
            let mut outer = boxed.shape;
            for h in &mut outer.half {
                *h += f32_f64(hw);
            }
            for r in &mut outer.radii {
                if *r > 0.0 {
                    *r += f32_f64(hw);
                } else {
                    // `analytic_join` routed only round joins and kept
                    // miters here; both draw the offset shape's corner.
                    *r = if stroke.join == kurbo::Join::Round {
                        f32_f64(hw)
                    } else {
                        0.0
                    };
                }
            }
            let mut inner = boxed.shape;
            let has_inner = inner.half.iter().all(|h| *h > f32_f64(hw));
            let inner_opt = if has_inner {
                for h in &mut inner.half {
                    *h -= f32_f64(hw);
                }
                for r in &mut inner.radii {
                    *r = (*r - f32_f64(hw)).max(0.0);
                }
                Some(inner)
            } else {
                None
            };
            let flags = if has_inner { FLAG_HAS_INNER } else { 0 };
            ops.push(Op::Shaped {
                kind: KIND_STROKE_OFFSET,
                local: ambient * boxed.extra,
                ambient,
                shape: outer,
                inner: inner_opt,
                bounds: boxed.bounds,
                extra_margin: hw,
                paint: resolve(paint, boxed.extra.inverse(), self.images)?,
                param_x: f32_f64(hw),
                flags,
            });
        } else {
            ops.push(Op::Shaped {
                kind: KIND_STROKE_DIST,
                local: ambient * boxed.extra,
                ambient,
                shape: boxed.shape,
                inner: None,
                bounds: boxed.bounds,
                extra_margin: hw,
                paint: resolve(paint, boxed.extra.inverse(), self.images)?,
                param_x: f32_f64(hw),
                flags: 0,
            });
        }
        Ok(())
    }

    /// A stroked line as a box in the line's local frame.
    fn stroke_line(
        &self,
        ambient: Affine,
        line: &Line,
        hw: f64,
        stroke: &kurbo::Stroke,
        paint: &Paint,
        ops: &mut Vec<Op>,
    ) -> Result<(), RenderError> {
        let d = line.p1 - line.p0;
        let len = d.hypot();
        if len <= 0.0 || hw <= 0.0 {
            return Ok(());
        }
        let mid = line.p0 + d * 0.5;
        let direction = d / len;
        let extra = Affine::new([
            direction.x,
            direction.y,
            -direction.y,
            direction.x,
            mid.x,
            mid.y,
        ]);
        let half = match stroke.start_cap {
            kurbo::Cap::Butt => [f32_f64(len / 2.0), f32_f64(hw)],
            _ => [f32_f64(len / 2.0 + hw), f32_f64(hw)],
        };
        let radii = if stroke.start_cap == kurbo::Cap::Round {
            [f32_f64(hw); 4]
        } else {
            [0.0; 4]
        };
        let boxed = Boxed {
            extra,
            shape: Shape {
                half,
                aspect: 1.0,
                exponent: 2.0,
                radii,
            },
            bounds: rect_around_origin(half),
        };
        ops.push(Op::Shaped {
            kind: KIND_FILL,
            local: ambient * boxed.extra,
            ambient,
            shape: boxed.shape,
            inner: None,
            bounds: boxed.bounds,
            extra_margin: 0.0,
            paint: resolve(paint, boxed.extra.inverse(), self.images)?,
            param_x: 0.0,
            flags: 0,
        });
        Ok(())
    }

    /// `Shadow`: a Gaussian-blurred rounded box, offset and spread.
    fn shadow(
        &self,
        ambient: Affine,
        shape: &ShapeData,
        shadow: &cherenkov::Shadow,
        ops: &mut Vec<Op>,
    ) -> Result<(), RenderError> {
        cherenkov::lowering::shadow::check_sigma(shadow.sigma)?;
        cherenkov::lowering::shadow::check_sigma(shadow.sigma)?;
        let boxed = match shape {
            // A line has no fillable silhouette to capture.
            ShapeData::Line(_) => return Err(RenderError::Unsupported(names::SHADOW)),
            ShapeData::Path { .. } => None,
            _ => box_shape(shape)?,
        };
        let Some(boxed) = boxed.filter(|boxed| shape_is_offsettable(boxed.shape)) else {
            if !shadow.offset.x.is_finite() || !shadow.offset.y.is_finite() {
                return Err(RenderError::Render("non-finite shadow offset".into()));
            }
            let start = ops.len();
            ops.push(Op::BeginShadow {
                parameters: super::shadow::Parameters {
                    transform: ambient,
                    sigma: shadow.sigma,
                    spread: shadow.spread,
                },
                end: 0,
            });
            self.fill(
                ambient * Affine::translate(shadow.offset),
                shape,
                &Paint::Solid(shadow.color),
                ops,
                None,
            )?;
            let end = u32::try_from(ops.len())
                .map_err(|_| RenderError::Render("shadow operation count exceeds u32".into()))?;
            let Op::BeginShadow { end: paired, .. } = &mut ops[start] else {
                unreachable!()
            };
            *paired = end;
            ops.push(Op::End);
            return Ok(());
        };
        let mut s = boxed.shape;
        let spread = f32_f64(shadow.spread);
        for h in &mut s.half {
            *h += spread;
        }
        if s.half.iter().any(|h| *h <= 0.0) {
            return Ok(());
        }
        for r in &mut s.radii {
            if *r > 0.0 {
                *r = (*r + spread).max(0.0);
            }
        }
        ops.push(Op::Shadow {
            local: ambient * Affine::translate(shadow.offset) * boxed.extra,
            ambient,
            shape: s,
            bounds: rect_around_origin(s.half),
            sigma_eff: shadow_sigma(shadow.sigma),
            color: shadow.color.components,
        });
        Ok(())
    }

    /// `Glyphs`: plain glyphs collect into `Glyphs` ops; COLR glyphs
    /// expand their picture at the glyph's placement, in order.
    fn glyph_run(
        &mut self,
        ambient: Affine,
        run: &GlyphRun,
        paint: &Paint,
        ops: &mut Vec<Op>,
        source: Option<usize>,
    ) -> Result<(), RenderError> {
        if let GlyphStyle::Stroke(style) = &run.style {
            return self.stroked_glyph_run(ambient, run, style, paint, ops);
        }
        let font = self
            .fonts
            .get(&run.font.raw())
            .ok_or_else(|| RenderError::Font(format!("unregistered font {:?}", run.font)))?;
        let resolved = resolve(paint, Affine::IDENTITY, self.images)?;
        if !font.has_colr
            && !font.has_bitmap
            && !run.glyphs.iter().any(|glyph| glyph.transform.is_some())
        {
            if !run.glyphs.is_empty() {
                ops.push(Op::Glyphs {
                    local: ambient,
                    bounds: super::glyph::run_bounds(font, run)?,
                    run: source.map_or_else(
                        || GlyphSource::Run(run.clone()),
                        |index| GlyphSource::Command {
                            index,
                            count: run.glyphs.len(),
                        },
                    ),
                    paint: resolved,
                });
            }
            return Ok(());
        }
        self.color_glyph_run(ambient, run, paint, ops, font, resolved)
    }

    #[cold]
    #[inline(never)]
    fn stroked_glyph_run(
        &self,
        ambient: Affine,
        run: &GlyphRun,
        style: &kurbo::Stroke,
        paint: &Paint,
        ops: &mut Vec<Op>,
    ) -> Result<(), RenderError> {
        let font = self
            .fonts
            .get(&run.font.raw())
            .ok_or_else(|| RenderError::Font(format!("unregistered font {:?}", run.font)))?;
        if font.has_bitmap {
            return Err(RenderError::Unsupported(names::GLYPH_STROKE));
        }
        for path in super::glyph::stroke_outlines(font, run)? {
            self.stroke(
                ambient,
                &ShapeData::Path {
                    elements: path.into_elements().into(),
                    rule: FillRule::NonZero,
                },
                style,
                paint,
                ops,
                None,
            )?;
        }
        Ok(())
    }

    /// Flushes accumulated plain glyphs as one `Op::Glyphs`.
    fn push_pending(
        ops: &mut Vec<Op>,
        ambient: Affine,
        run: &GlyphRun,
        pending: &mut Vec<cherenkov::Glyph>,
        paint: ResolvedPaint,
        font: &FontData,
    ) -> Result<(), RenderError> {
        if pending.is_empty() {
            return Ok(());
        }
        let run = GlyphRun {
            font: run.font,
            size: run.size,
            coords: run.coords.clone(),
            glyphs: std::mem::take(pending).into(),
            style: run.style.clone(),
        };
        ops.push(Op::Glyphs {
            local: ambient,
            bounds: super::glyph::run_bounds(font, &run)?,
            run: GlyphSource::Run(run),
            paint,
        });
        Ok(())
    }

    #[inline(never)]
    fn color_glyph_run(
        &mut self,
        ambient: Affine,
        run: &GlyphRun,
        paint: &Paint,
        ops: &mut Vec<Op>,
        font: &FontData,
        resolved: ResolvedPaint,
    ) -> Result<(), RenderError> {
        let font_ref = skrifa::FontRef::from_index(&font.data, font.index)
            .map_err(|e| RenderError::Font(format!("{e}")))?;
        let upem = f64::from(
            font_ref
                .head()
                .map_err(|e| RenderError::Font(format!("head: {e}")))?
                .units_per_em(),
        );
        if upem <= 0.0 {
            return Err(RenderError::Font("zero units_per_em".into()));
        }
        let s = f64::from(run.size) / upem;
        let font_scale = Affine::scale_non_uniform(s, -s);
        let coords: Vec<F2Dot14> = run.coords.iter().map(|c| F2Dot14::from_bits(*c)).collect();
        let outlines = font_ref.outline_glyphs();
        let color_glyphs =
            (font.has_colr && font_ref.colr().is_ok()).then(|| font_ref.color_glyphs());
        let bitmap = font.has_bitmap;
        let mut pending: Vec<cherenkov::Glyph> = Vec::new();
        for glyph in run.glyphs.iter() {
            if let Some(color_glyphs) = color_glyphs.as_ref()
                && color_glyphs.get(skrifa::GlyphId::new(glyph.id)).is_some()
            {
                Self::push_pending(ops, ambient, run, &mut pending, resolved.clone(), font)?;
                let picture = crate::render::colr::glyph_picture(
                    font,
                    run.font.raw(),
                    glyph.id,
                    &run.coords,
                    paint,
                    self.pending,
                )?;
                // `translate(x, y) * transform * scale_non_uniform(size/upem,
                // -size/upem)` places the font-space picture at the
                // glyph's origin.
                let place = Affine::translate((f64::from(glyph.x), f64::from(glyph.y)))
                    * super::glyph::checked_transform(glyph)?
                    * font_scale;
                let mut lowerer = Lowerer {
                    fonts: self.fonts,
                    images: self.images,
                    pending: self.pending,
                };
                cherenkov::lowering::append(
                    picture.display_list(),
                    ambient * place,
                    &mut lowerer,
                    ops,
                )?;
                continue;
            }
            if bitmap {
                // The placement composes like an image drawn under the
                // glyph's transform, so the two realize identically.
                let place = match super::glyph::classify(glyph)? {
                    super::glyph::GlyphPlacement::Translate(glyph) => {
                        Affine::translate((f64::from(glyph.x), f64::from(glyph.y)))
                    }
                    super::glyph::GlyphPlacement::Outline(place) => place,
                };
                ops.push(Op::BitmapGlyph {
                    local: ambient * place,
                    font: run.font.raw(),
                    glyph: glyph.id,
                    size: run.size,
                });
                continue;
            }
            let place = match super::glyph::classify(glyph)? {
                super::glyph::GlyphPlacement::Translate(g) => {
                    pending.push(g);
                    continue;
                }
                super::glyph::GlyphPlacement::Outline(place) => place,
            };
            let Some(path) = super::glyph::outline(&outlines, &coords, glyph.id)? else {
                return Err(RenderError::Font(format!(
                    "glyph {} has no outline",
                    glyph.id
                )));
            };
            if path.elements().is_empty() {
                continue;
            }
            Self::push_pending(ops, ambient, run, &mut pending, resolved.clone(), font)?;
            self.fill(
                ambient,
                &ShapeData::Path {
                    elements: (place * font_scale * path).into_elements().into(),
                    rule: FillRule::NonZero,
                },
                paint,
                ops,
                None,
            )?;
        }
        Self::push_pending(ops, ambient, run, &mut pending, resolved, font)?;
        Ok(())
    }
}

/// The clip shape a `BeginClip` lowers to.
fn clip_shape(shape: &ShapeData) -> Result<ClipShape, RenderError> {
    if let ShapeData::Path { elements, rule } = shape {
        return Ok(ClipShape::Path {
            elements: Arc::clone(elements),
            rule: *rule,
        });
    }
    let Some(boxed) = box_shape(shape)? else {
        return Ok(ClipShape::Empty);
    };
    Ok(ClipShape::Boxed {
        extra: boxed.extra,
        shape: boxed.shape,
        rect: match shape {
            ShapeData::Rect(r) => Some(*r),
            _ => None,
        },
    })
}

#[cfg(test)]
mod tests {
    use cherenkov::WorkingColor;
    use cherenkov::kurbo::Ellipse;

    use super::*;

    /// `stroke` against empty registries, as `draw` dispatches a
    /// `Command::Stroke`.
    fn lower_stroke(shape: &ShapeData, stroke: &kurbo::Stroke) -> Result<(), RenderError> {
        let (mut pending, fonts, images) = (Vec::new(), FxHashMap::default(), FxHashMap::default());
        let lowerer = Lowerer {
            pending: &mut pending,
            fonts: &fonts,
            images: &images,
        };
        let mut ops = Vec::new();
        lowerer.stroke(
            Affine::IDENTITY,
            shape,
            stroke,
            &Paint::Solid(WorkingColor::WHITE),
            &mut ops,
            None,
        )
    }

    /// One shape per route `stroke` selects: the general outline path, the
    /// line box path, and the offset and distance analytic strokes.
    fn route_shapes() -> [ShapeData; 4] {
        let mut path = kurbo::BezPath::new();
        path.move_to((4.0, 4.0));
        path.line_to((12.0, 12.0));
        [
            ShapeData::Path {
                elements: path.into_elements().into(),
                rule: FillRule::NonZero,
            },
            ShapeData::Line(Line::new((0.0, 0.0), (8.0, 0.0))),
            ShapeData::Rect(Rect::new(0.0, 0.0, 8.0, 8.0)),
            ShapeData::Ellipse(Ellipse::new((4.0, 4.0), (8.0, 3.0), 0.0)),
        ]
    }

    /// A negative or non-finite dash entry is rejected before the lowering
    /// picks a route, so every shape reports the same error.
    #[test]
    fn a_negative_or_nonfinite_dash_entry_errors_on_every_route() {
        for entry in [-1.0, f64::NEG_INFINITY, f64::NAN, f64::INFINITY] {
            let mut stroke = kurbo::Stroke::new(2.0);
            stroke.dash_pattern.extend([4.0, entry]);
            for shape in route_shapes() {
                assert!(
                    matches!(lower_stroke(&shape, &stroke), Err(RenderError::Render(_))),
                    "dash entry {entry} on {shape:?}"
                );
            }
        }
    }

    /// A dash pattern whose entries sum to zero is rejected on every
    /// route.
    #[test]
    fn a_dash_pattern_summing_to_zero_errors_on_every_route() {
        let mut stroke = kurbo::Stroke::new(2.0);
        stroke.dash_pattern.extend([0.0, 0.0]);
        for shape in route_shapes() {
            assert!(
                matches!(lower_stroke(&shape, &stroke), Err(RenderError::Render(_))),
                "zero-sum pattern on {shape:?}"
            );
        }
    }

    /// A non-finite dash offset is rejected on every route, dashed or not.
    #[test]
    fn a_nonfinite_dash_offset_errors_on_every_route() {
        for offset in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for pattern in [Vec::new(), vec![4.0, 2.0]] {
                let mut stroke = kurbo::Stroke::new(2.0);
                stroke.dash_offset = offset;
                stroke.dash_pattern.extend(pattern.iter().copied());
                for shape in route_shapes() {
                    assert!(
                        matches!(lower_stroke(&shape, &stroke), Err(RenderError::Render(_))),
                        "dash offset {offset}, {pattern:?} on {shape:?}"
                    );
                }
            }
        }
    }
}
