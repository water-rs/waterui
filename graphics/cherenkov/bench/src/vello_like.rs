//! Shared scene lowering + replay for the vello adapters
//! (`vello_classic`, `vello_cpu`, `vello_hybrid`).
//!
//! [`lower`] runs in `prepare`: it walks the layer tree once and records
//! an [`Op`] list whose items own their resolved `BezPath`, engine paint,
//! transform, clip and glyph data. `encode` then replays the list — the
//! engine's recording calls only. `vello_cpu` and `vello_hybrid` share
//! [`replay`] through the [`VelloLikeCtx`] surface (`vello_classic`
//! replays its own `Op<peniko::Brush>` list against `vello::Scene`).

use cherenkov_scene::{
    BlendMode, Draw, Feature, Glyph, Item, Layer, Paint, ResourceHash, Scene, Shape,
};
use kurbo::{Affine, BezPath, Rect, Shape as _, Stroke};
use peniko::{BlendMode as PBlendMode, Fill, FontData};
use vello_common::paint::PaintType;

use crate::convert::{self, Prepared};
use crate::{BenchError, Counters};

/// The `RenderContext`-style surface shared by `vello_cpu` and
/// `vello_hybrid::Scene`.
pub trait VelloLikeCtx {
    /// Absolute user-space transform.
    fn set_transform(&mut self, t: Affine);
    /// The paint used by subsequent fills.
    fn set_paint(&mut self, p: PaintType);
    /// Paint-space transform (image patterns).
    fn set_paint_transform(&mut self, t: Affine);
    /// Identity paint transform.
    fn reset_paint_transform(&mut self);
    /// Fill rule for `fill_path`.
    fn set_fill_rule(&mut self, f: Fill);
    /// Stroke style for `stroke_path`.
    fn set_stroke(&mut self, s: Stroke);
    /// Fill a path.
    fn fill_path(&mut self, p: &BezPath);
    /// Stroke a path.
    fn stroke_path(&mut self, p: &BezPath);
    /// Fill a rect with the current paint (used for `Draw::Image`).
    fn fill_rect(&mut self, r: &Rect);
    /// Push a clipped / blended / group-opacity layer.
    fn push_layer(
        &mut self,
        clip: Option<&BezPath>,
        blend: Option<PBlendMode>,
        opacity: Option<f32>,
    );
    /// Pop the matching layer.
    fn pop_layer(&mut self);
    /// `vello`'s blurred-rounded-rect primitive (only shape shadows it can
    /// express: axis-aligned rects and uniform-radius rounded rects).
    fn fill_blurred_rrect(&mut self, rect: &Rect, radius: f32, std_dev: f32);
    /// Fill a glyph run with the current paint.
    fn draw_glyphs_fill(&mut self, font: &FontData, size: f32, coords: &[i16], glyphs: Vec<Glyph>);
}

/// The upstream API vello-family adapters lack for non-rectilinear shadows.
///
/// The vello blurred primitive (`RenderContext::fill_blurred_rounded_rect` /
/// `vello::Scene::draw_blurred_rounded_rect`) takes a rect plus a single
/// uniform corner radius.
pub const SHADOW_SHAPE_API: &str =
    "vello blurred rounded rect takes a rect plus a single uniform corner radius";

/// Maps a shadow [`Shape`] to the `(rect, uniform_radius)` the vello blurred
/// rounded-rect primitive can express, or `None`.
///
/// The primitive takes a rect and a single uniform corner radius, so it can
/// express exactly rects (radius 0), uniform-radius rounded rects, circles,
/// and circles expressed as `Ellipse` — a square rrect with radius = half
/// its side is a circle. Non-uniform rounded rects, non-circular ellipses,
/// continuous corners and arbitrary paths are not expressible.
#[must_use]
pub fn expressible_shadow(shape: &Shape) -> Option<(Rect, f64)> {
    match shape {
        Shape::Rect(rect) => Some((*rect, 0.0)),
        Shape::Circle(c) => {
            let r = c.radius;
            Some((
                Rect::new(
                    c.center.x - r,
                    c.center.y - r,
                    c.center.x + r,
                    c.center.y + r,
                ),
                r,
            ))
        }
        Shape::Ellipse(e) if (e.radii().x - e.radii().y).abs() < f64::EPSILON => {
            let r = e.radii().x;
            let c = e.center();
            Some((Rect::new(c.x - r, c.y - r, c.x + r, c.y + r), r))
        }
        Shape::RoundedRect(rrect) => {
            let radii = rrect.radii();
            let uniform = (radii.top_left - radii.top_right).abs() < f64::EPSILON
                && (radii.top_left - radii.bottom_right).abs() < f64::EPSILON
                && (radii.top_left - radii.bottom_left).abs() < f64::EPSILON;
            uniform.then(|| (rrect.rect(), radii.top_left))
        }
        _ => None,
    }
}

/// The upstream API a vello-family adapter lacks for a declared scene
/// feature. Answers the "why" an `UnsupportedReport` records; `None` when
/// the adapter simply does not map the feature yet.
#[must_use]
pub const fn vello_missing_api(f: &Feature) -> Option<&'static str> {
    match f {
        Feature::ExtendNone => Some(crate::convert::EXTEND_NONE_API),
        Feature::HdrColor | Feature::WideGamut => {
            Some("vello-family render targets are rgba8unorm sRGB — no HDR or wide-gamut output")
        }
        Feature::InterpolationSpace(_) => Some(crate::convert::LINEAR_P3_API),
        _ => None,
    }
}

/// The features a `vello`-family `RenderContext` adapter executes
/// faithfully.
///
/// [`Feature::Shadow`] is supported only for rects, uniform-radius rounded
/// rects, and circles (a circle is a square rounded rect with radius = half
/// the side) via `fill_blurred_rrect`; the per-shape check happens in the
/// draw walk. [`Feature::ExtendNone`] is absent: `peniko::Extend` has no
/// `None` variant. [`Feature::HdrColor`] and [`Feature::WideGamut`] are
/// absent: these adapters render into an `rgba8unorm` sRGB target and
/// clamp, so they honestly report HDR/wide-gamut scenes unsupported rather
/// than silently quantizing them.
///
/// Gradient interpolation is honest too: every space `peniko`'s
/// `ColorSpaceTag` names is claimed; `linear-p3` is not among them and is
/// reported unsupported instead of remapped.
#[must_use]
pub fn vello_features() -> Vec<Feature> {
    let mut v = vec![
        Feature::Fill,
        Feature::EvenOdd,
        Feature::Stroke,
        Feature::StrokeDash,
        Feature::Path,
        Feature::ContinuousCorners,
        Feature::LinearGradient,
        Feature::RadialGradient,
        Feature::SweepGradient,
        Feature::Image,
        Feature::ImagePaint,
        Feature::Clip,
        Feature::Opacity,
        Feature::Scroll,
        Feature::Shadow,
        Feature::Glyphs,
        Feature::FontVariations,
        // Interpolation spaces `peniko::color::ColorSpaceTag` covers —
        // `ColorSpace::LinearP3` has no tag and stays unsupported.
        Feature::InterpolationSpace(cherenkov_scene::ColorSpace::Srgb),
        Feature::InterpolationSpace(cherenkov_scene::ColorSpace::LinearSrgb),
        Feature::InterpolationSpace(cherenkov_scene::ColorSpace::DisplayP3),
        Feature::InterpolationSpace(cherenkov_scene::ColorSpace::Rec2020),
    ];
    for m in BlendMode::ALL {
        v.push(Feature::Blend(m));
    }
    v
}

/// One recording step of a lowered scene.
///
/// Every `Shape` is already a resolved `BezPath`, every paint the
/// engine's own paint value, every glyph run its font + resolved
/// `F2Dot14` coords + positions — the timed `encode` replays this list
/// and covers only the engine's recording calls.
///
/// `B` is the engine's paint payload: `peniko::Brush` for `vello`
/// classic, `vello_common::paint::PaintType` for the `RenderContext`
/// engines.
#[derive(Debug)]
pub enum Op<B> {
    /// Push a clipped / blended / group-opacity layer.
    PushLayer {
        /// Accumulated user→device transform under which the clip applies.
        transform: Affine,
        /// Clip path; `None` only for the `RenderContext` engines —
        /// `lower` substitutes a near-infinite rect when `opaque_clip` is
        /// set (vello classic's `push_layer` requires a clip path).
        clip: Option<BezPath>,
        /// Blend mode of the group.
        blend: PBlendMode,
        /// Group opacity.
        opacity: f32,
    },
    /// Pop the matching layer.
    PopLayer,
    /// `Draw::Fill`.
    Fill {
        /// Accumulated user→device transform.
        transform: Affine,
        /// Fill rule.
        rule: Fill,
        /// Engine-resolved paint.
        paint: B,
        /// Paint-space transform (image patterns), if any.
        paint_transform: Option<Affine>,
        /// Resolved shape path.
        path: BezPath,
    },
    /// `Draw::Stroke` (stroke style resolved to `kurbo::Stroke`).
    Stroke {
        /// Accumulated user→device transform.
        transform: Affine,
        /// Stroke style.
        stroke: Stroke,
        /// Engine-resolved paint.
        paint: B,
        /// Paint-space transform (image patterns), if any.
        paint_transform: Option<Affine>,
        /// Resolved shape path.
        path: BezPath,
    },
    /// `Draw::Image`: fill `dst` (pre-resolved as `dst_path`) with an
    /// image paint sampled under `paint_transform`.
    ImageFill {
        /// Accumulated user→device transform.
        transform: Affine,
        /// Engine-resolved image paint.
        paint: B,
        /// Image→`dst` sampling transform.
        paint_transform: Affine,
        /// Destination rect.
        dst: Rect,
        /// `dst` as a resolved path (vello classic fills paths).
        dst_path: BezPath,
    },
    /// `Draw::Shadow` for a shape [`expressible_shadow`] can express —
    /// a rect plus a single uniform corner radius.
    Shadow {
        /// Accumulated user→device transform including the shadow offset.
        transform: Affine,
        /// Blurred rect.
        rect: Rect,
        /// Uniform corner radius.
        radius: f64,
        /// Gaussian blur sigma.
        std_dev: f64,
        /// Shadow colour.
        color: peniko::color::AlphaColor<peniko::color::Srgb>,
    },
    /// `Draw::Glyphs`: `peniko::FontData`, resolved variation coords and
    /// glyph positions carried wholesale.
    Glyphs {
        /// Accumulated user→device transform.
        transform: Affine,
        /// Engine-resolved paint.
        paint: B,
        /// Paint-space transform (image patterns), if any.
        paint_transform: Option<Affine>,
        /// Font data handle.
        font: FontData,
        /// Em size.
        size: f32,
        /// Resolved `F2Dot14` design-axis bits, in font axis order.
        coords: Vec<i16>,
        /// Glyph ids and positions.
        glyphs: Vec<Glyph>,
    },
}

/// A scene lowered once in `prepare` — replayable recording ops plus
/// bookkeeping that is identical every frame.
#[derive(Debug)]
pub struct Lowered<B> {
    /// Recording ops in draw order.
    pub ops: Vec<Op<B>>,
    /// Total layers visited (the `layers` counter counts every layer,
    /// including ones without clip/blend/opacity that emit no op).
    pub layers: u32,
}

/// Lowers `scene` into a [`Lowered`] op list, in `prepare`.
///
/// Shape→path conversion (including `ContinuousRect`'s adaptive
/// subdivision and arc fitting), paint resolution, font/coord lookup and
/// image-payload resolution all happen here, not in the timed `encode`.
///
/// `paint_fn` resolves a [`Paint`] to the engine's paint value;
/// `image_fn` does the same for a `Draw::Image` payload given its
/// resource hash and declared sampling. `opaque_clip` substitutes a
/// near-infinite rect path for a clip-less grouped layer.
///
/// # Errors
/// [`BenchError::Unsupported`] for a shadow shape exceeding
/// [`expressible_shadow`]; a `BenchError` from the paint fns or a missing
/// resource.
pub fn lower<B>(
    scene: &Scene,
    prepared: &Prepared,
    engine: &'static str,
    opaque_clip: bool,
    paint_fn: &dyn Fn(&Paint) -> Result<B, BenchError>,
    image_fn: &dyn Fn(ResourceHash, cherenkov_scene::Sampling) -> Result<B, BenchError>,
) -> Result<Lowered<B>, BenchError> {
    let mut lowered = Lowered {
        ops: Vec::new(),
        layers: 0,
    };
    lower_layer(
        &mut lowered,
        &scene.root,
        Affine::IDENTITY,
        prepared,
        engine,
        opaque_clip,
        paint_fn,
        image_fn,
    )?;
    Ok(lowered)
}

#[allow(clippy::too_many_arguments)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "vello layer opacity is f32 at the engine boundary; scene values are f64"
)]
fn lower_layer<B>(
    lowered: &mut Lowered<B>,
    layer: &Layer,
    parent_tf: Affine,
    prepared: &Prepared,
    engine: &'static str,
    opaque_clip: bool,
    paint_fn: &dyn Fn(&Paint) -> Result<B, BenchError>,
    image_fn: &dyn Fn(ResourceHash, cherenkov_scene::Sampling) -> Result<B, BenchError>,
) -> Result<(), BenchError> {
    lowered.layers += 1;
    let tf = parent_tf * layer.transform;
    let grouped = layer.clip.is_some() || layer.blend != BlendMode::Normal || layer.opacity < 1.0;
    if grouped {
        let clip = layer.clip.as_ref().map_or_else(
            || opaque_clip.then(|| Rect::new(0.0, 0.0, 1e9, 1e9).to_path(0.25)),
            |c| Some(convert::shape_path(c)),
        );
        lowered.ops.push(Op::PushLayer {
            transform: tf,
            clip,
            blend: convert::blend(layer.blend),
            opacity: layer.opacity as f32,
        });
    }
    // Content and children draw translated by -scroll_offset inside the
    // clip; `motion` is unsupported (not in `vello_features`).
    let content_tf = tf * Affine::translate((-layer.scroll_offset.x, -layer.scroll_offset.y));
    for item in &layer.items {
        match item {
            Item::Layer(l) => lower_layer(
                lowered,
                l,
                content_tf,
                prepared,
                engine,
                opaque_clip,
                paint_fn,
                image_fn,
            )?,
            Item::Draw(d) => {
                lower_draw(lowered, d, content_tf, prepared, engine, paint_fn, image_fn)?;
            }
            // Unreachable: scenes declaring `blend-space` fail the feature
            // check; a group with no declared feature needs no support.
            Item::Group(_) => {
                return Err(BenchError::Unsupported {
                    engine,
                    feature: Feature::BlendSpace(cherenkov_scene::BlendSpace::SrgbEncoded),
                    api: Some("vello: scoped group compositing"),
                });
            }
        }
    }
    if grouped {
        lowered.ops.push(Op::PopLayer);
    }
    Ok(())
}

fn lower_draw<B>(
    lowered: &mut Lowered<B>,
    draw: &Draw,
    tf: Affine,
    prepared: &Prepared,
    engine: &'static str,
    paint_fn: &dyn Fn(&Paint) -> Result<B, BenchError>,
    image_fn: &dyn Fn(ResourceHash, cherenkov_scene::Sampling) -> Result<B, BenchError>,
) -> Result<(), BenchError> {
    let op = match draw {
        Draw::Fill { shape, rule, paint } => Op::Fill {
            transform: tf,
            rule: convert::fill(*rule),
            paint: paint_fn(paint)?,
            paint_transform: convert::brush_transform(paint),
            path: convert::shape_path(shape),
        },
        Draw::Stroke {
            shape,
            stroke,
            paint,
        } => Op::Stroke {
            transform: tf,
            stroke: convert::stroke(stroke),
            paint: paint_fn(paint)?,
            paint_transform: convert::brush_transform(paint),
            path: convert::shape_path(shape),
        },
        Draw::Image {
            image,
            dst,
            sampling,
            ..
        } => {
            let data = prepared.image(*image)?;
            Op::ImageFill {
                transform: tf,
                paint: image_fn(*image, *sampling)?,
                paint_transform: convert::image_draw_transform(data.width, data.height, *dst),
                dst: *dst,
                dst_path: dst.to_path(0.25),
            }
        }
        Draw::Glyphs(run) => Op::Glyphs {
            transform: tf,
            paint: paint_fn(&run.paint)?,
            paint_transform: convert::brush_transform(&run.paint),
            font: prepared.font(run.font, run.font_index)?,
            size: run.size,
            coords: prepared.coord_bits(run.font, &run.normalized_coords),
            glyphs: run.glyphs.clone(),
        },
        Draw::Shadow {
            shape,
            blur_sigma,
            offset,
            color,
        } => {
            let Some((rect, radius)) = expressible_shadow(shape) else {
                return Err(BenchError::Unsupported {
                    engine,
                    feature: Feature::Shadow,
                    api: Some(SHADOW_SHAPE_API),
                });
            };
            Op::Shadow {
                transform: tf * Affine::translate(kurbo::Vec2::new(offset[0], offset[1])),
                rect,
                radius,
                std_dev: *blur_sigma,
                color: convert::peniko_solid(color),
            }
        }
    };
    lowered.ops.push(op);
    Ok(())
}

/// Replays a [`Lowered`] op list into a `VelloLikeCtx` — the timed
/// `encode` of `vello-cpu` and `vello-hybrid` is exactly this loop: the
/// engine's recording calls, nothing else.
pub fn replay<C: VelloLikeCtx>(ctx: &mut C, lowered: &Lowered<PaintType>, counters: &mut Counters) {
    counters.layers = lowered.layers;
    for op in &lowered.ops {
        replay_op(ctx, op, counters);
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "vello blurred-rrect radii are f32 at the engine boundary; scene geometry is f64"
)]
fn replay_op<C: VelloLikeCtx>(ctx: &mut C, op: &Op<PaintType>, counters: &mut Counters) {
    match op {
        Op::PushLayer {
            transform,
            clip,
            blend,
            opacity,
        } => {
            ctx.set_transform(*transform);
            ctx.push_layer(clip.as_ref(), Some(*blend), Some(*opacity));
        }
        Op::PopLayer => ctx.pop_layer(),
        Op::Fill {
            transform,
            rule,
            paint,
            paint_transform,
            path,
        } => {
            counters.draw_commands += 1;
            ctx.set_transform(*transform);
            ctx.set_fill_rule(*rule);
            apply_paint(ctx, paint, *paint_transform);
            ctx.fill_path(path);
        }
        Op::Stroke {
            transform,
            stroke,
            paint,
            paint_transform,
            path,
        } => {
            counters.draw_commands += 1;
            ctx.set_transform(*transform);
            ctx.set_fill_rule(Fill::NonZero);
            ctx.set_stroke(stroke.clone());
            apply_paint(ctx, paint, *paint_transform);
            ctx.stroke_path(path);
        }
        Op::ImageFill {
            transform,
            paint,
            paint_transform,
            dst,
            ..
        } => {
            counters.draw_commands += 1;
            ctx.set_transform(*transform);
            ctx.set_fill_rule(Fill::NonZero);
            ctx.set_paint(paint.clone());
            ctx.set_paint_transform(*paint_transform);
            ctx.fill_rect(dst);
            ctx.reset_paint_transform();
        }
        Op::Glyphs {
            transform,
            paint,
            paint_transform,
            font,
            size,
            coords,
            glyphs,
        } => {
            counters.draw_commands += 1;
            ctx.set_transform(*transform);
            apply_paint(ctx, paint, *paint_transform);
            ctx.draw_glyphs_fill(font, *size, coords, glyphs.clone());
        }
        Op::Shadow {
            transform,
            rect,
            radius,
            std_dev,
            color,
        } => {
            counters.draw_commands += 1;
            ctx.set_transform(*transform);
            ctx.set_paint(PaintType::Solid(*color));
            ctx.fill_blurred_rrect(rect, *radius as f32, *std_dev as f32);
        }
    }
}

fn apply_paint<C: VelloLikeCtx>(ctx: &mut C, paint: &PaintType, transform: Option<Affine>) {
    ctx.set_paint(paint.clone());
    match transform {
        Some(t) => ctx.set_paint_transform(t),
        None => ctx.reset_paint_transform(),
    }
}
