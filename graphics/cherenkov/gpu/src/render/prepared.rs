//! Stage-1 lowering: a display list becomes a device-independent op
//! stream, patchable in place along [`cherenkov::Dirty`] ranges.

use std::sync::Arc;

use rustc_hash::FxHashMap;

use cherenkov::kurbo::{Affine, Line, PathEl, Rect};
use cherenkov::{
    BlendMode, BlendSpace, Command, Extend, FillRule, ImageId, ImagePattern, Interpolation, Paint,
    RenderError, ShapeData,
};
use cherenkov::{GlyphRun, GlyphStyle};

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

/// Linear Display P3 to linear sRGB (the inverse of the shader's
/// `SRGB_TO_P3`), used to store `SrgbEncoded` gradient stops.
const P3_TO_SRGB: [[f32; 3]; 3] = [
    [1.224_940_1, -0.224_940_4, 0.0],
    [-0.042_056_9, 1.042_057_1, 0.0],
    [-0.019_637_6, -0.078_636_1, 1.098_273_5],
];

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
/// transform that centres it.
///
/// A `Line` has no area and draws nothing, so it returns `None`.
pub fn box_shape(shape: &ShapeData) -> Result<Option<Boxed>, RenderError> {
    let boxed = match shape {
        ShapeData::Rect(r) => {
            let half = [f32_f64(r.width() / 2.0), f32_f64(r.height() / 2.0)];
            Boxed {
                extra: Affine::translate(r.center().to_vec2()),
                shape: Shape::rect(half),
                bounds: rect_around_origin(half),
            }
        }
        ShapeData::RoundedRect(rr) => {
            let r = rr.rect();
            let half = [f32_f64(r.width() / 2.0), f32_f64(r.height() / 2.0)];
            let radii = clamped_radii(rr.radii(), half);
            Boxed {
                extra: Affine::translate(r.center().to_vec2()),
                shape: Shape {
                    half,
                    aspect: 1.0,
                    exponent: 2.0,
                    radii,
                },
                bounds: rect_around_origin(half),
            }
        }
        ShapeData::Continuous(c) => {
            let r = c.rect;
            let half = [f32_f64(r.width() / 2.0), f32_f64(r.height() / 2.0)];
            let radii = clamped_radii(c.radii, half);
            Boxed {
                extra: Affine::translate(r.center().to_vec2()),
                shape: Shape {
                    half,
                    aspect: 1.0,
                    exponent: f32_f64(c.smoothing).clamp(0.0, 1.0).mul_add(2.0, 2.0),
                    radii,
                },
                bounds: rect_around_origin(half),
            }
        }
        ShapeData::Circle(c) => {
            let r = c.radius;
            if r <= 0.0 {
                return Ok(None);
            }
            let half = [f32_f64(r), f32_f64(r)];
            Boxed {
                extra: Affine::translate(c.center.to_vec2()),
                shape: Shape {
                    half,
                    aspect: 1.0,
                    exponent: 2.0,
                    radii: [f32_f64(r); 4],
                },
                bounds: rect_around_origin(half),
            }
        }
        ShapeData::Ellipse(e) => {
            let radii_v = e.radii();
            let (a, b) = (radii_v.x, radii_v.y);
            if a <= 0.0 {
                return Ok(None);
            }
            let half = [f32_f64(a), f32_f64(b)];
            Boxed {
                extra: Affine::translate(e.center().to_vec2()) * Affine::rotate(e.rotation()),
                shape: Shape {
                    half,
                    aspect: f32_f64(b / a),
                    exponent: 2.0,
                    radii: [f32_f64(a); 4],
                },
                bounds: rect_around_origin(half),
            }
        }
        ShapeData::Line(_) => return Ok(None),
        ShapeData::Path { .. } => {
            return Err(RenderError::Unsupported(names::PATH));
        }
    };
    Ok(Some(boxed))
}

fn rect_around_origin(half: [f32; 2]) -> Rect {
    Rect::new(
        -f64::from(half[0]),
        -f64::from(half[1]),
        f64::from(half[0]),
        f64::from(half[1]),
    )
}

fn clamped_radii(radii: kurbo::RoundedRectRadii, half: [f32; 2]) -> [f32; 4] {
    let limit = f64::from(half[0].min(half[1]));
    [
        f32_f64(radii.top_left.clamp(0.0, limit)),
        f32_f64(radii.top_right.clamp(0.0, limit)),
        f32_f64(radii.bottom_right.clamp(0.0, limit)),
        f32_f64(radii.bottom_left.clamp(0.0, limit)),
    ]
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
    /// `extend_x | extend_y << 4 | sampling << 8`.
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

/// sRGB-encodes one channel, preserving sign.
fn srgb_encode(x: f32) -> f32 {
    let e = if x.abs() <= 0.003_130_8 {
        x.abs() * 12.92
    } else {
        1.055f32.mul_add(x.abs().powf(1.0 / 2.4), -0.055)
    };
    e.copysign(x)
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
            let [sr, sg, sb] = [
                P3_TO_SRGB[0][0].mul_add(r, P3_TO_SRGB[0][1].mul_add(g, P3_TO_SRGB[0][2] * b)),
                P3_TO_SRGB[1][0].mul_add(r, P3_TO_SRGB[1][1].mul_add(g, P3_TO_SRGB[1][2] * b)),
                P3_TO_SRGB[2][0].mul_add(r, P3_TO_SRGB[2][1].mul_add(g, P3_TO_SRGB[2][2] * b)),
            ];
            [srgb_encode(sr), srgb_encode(sg), srgb_encode(sb), a]
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

/// Lowers a paint; `to_local` maps content space to the instance's local
/// (shape-centred) space in which the shader evaluates gradient parameters.
#[expect(
    clippy::cast_precision_loss,
    clippy::many_single_char_names,
    reason = "image dimensions fit f32; affine coefficients are conventionally a..f"
)]
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
            data.kind = PAINT_RADIAL;
            let c0 = to_local * g.start_center;
            let c1 = to_local * g.end_center;
            data.grad = [f32_f64(c0.x), f32_f64(c0.y), f32_f64(c1.x), f32_f64(c1.y)];
            data.grad2 = [f32_f64(g.start_radius), f32_f64(g.end_radius), 0.0, 0.0];
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
            // `to_local * transform` maps image space to instance-local; the
            // shader needs the inverse.
            let [a, b, c, d, e, f] = (to_local * pattern.transform).inverse().as_coeffs();
            data.kind = PAINT_IMAGE;
            data.grad = [f32_f64(a), f32_f64(b), f32_f64(c), f32_f64(d)];
            let (iw, ih) = (img.width as f32, img.height as f32);
            data.grad2 = [f32_f64(e), f32_f64(f), iw, ih];
            let sampling = match pattern.sampling {
                cherenkov::Sampling::Nearest => 0,
                cherenkov::Sampling::Linear => 1,
            };
            data.packed = extend_code(pattern.extend_x)
                | (extend_code(pattern.extend_y) << 4)
                | (sampling << 8);
            data.image = Some(pattern.image.raw());
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
        /// The glyph origin in run coordinates.
        origin: [f32; 2],
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
        let (iw, ih) = (f64::from(img.width), f64::from(img.height));
        let (dw, dh) = (dst.x1 - dst.x0, dst.y1 - dst.y0);
        if dw <= 0.0 || dh <= 0.0 {
            return Ok(());
        }
        let transform =
            Affine::translate((dst.x0, dst.y0)) * Affine::scale_non_uniform(dw / iw, dh / ih);
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
        let extra = Affine::translate(mid.to_vec2()) * Affine::rotate(d.y.atan2(d.x));
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
    ) {
        if pending.is_empty() {
            return;
        }
        ops.push(Op::Glyphs {
            local: ambient,
            run: GlyphSource::Run(GlyphRun {
                font: run.font,
                size: run.size,
                coords: run.coords.clone(),
                glyphs: std::mem::take(pending).into(),
                style: run.style.clone(),
            }),
            paint,
        });
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
                Self::push_pending(ops, ambient, run, &mut pending, resolved.clone());
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
                let (local, origin) = match super::glyph::classify(glyph)? {
                    super::glyph::GlyphPlacement::Translate(glyph) => (ambient, [glyph.x, glyph.y]),
                    super::glyph::GlyphPlacement::Outline(place) => (ambient * place, [0.0, 0.0]),
                };
                ops.push(Op::BitmapGlyph {
                    local,
                    font: run.font.raw(),
                    glyph: glyph.id,
                    origin,
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
            Self::push_pending(ops, ambient, run, &mut pending, resolved.clone());
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
        Self::push_pending(ops, ambient, run, &mut pending, resolved);
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
