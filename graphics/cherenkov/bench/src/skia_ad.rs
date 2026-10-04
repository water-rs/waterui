//! Skia adapter: `skia-cpu` (raster surface), `skia-vulkan` (Ganesh on
//! Vulkan, Linux and Android only) and `skia-metal` (Graphite on Metal,
//! Apple platforms only).
//!
//! Colour: `skia-safe` renders into premultiplied `RGBAF16` surfaces whose
//! colour space is linear Display P3 (CICP primaries 12 / transfer 8) —
//! the suite working space — so HDR and wide-gamut values are carried
//! unclamped end to end and the `HdrColor`/`WideGamut` claims are real.
//! Readback is `f16` premultiplied linear P3 decoded to `f32`.
//!
//! Prepare/encode/submit split: `prepare` runs once per scene — feature
//! check, render-target creation, PNG decode, typeface/font creation,
//! gradient shaders — and converts the scene into a `Cmd` list of
//! pre-resolved Skia objects. The timed per-frame `encode` replays that
//! list: the `Canvas` draw calls themselves (Skia's recording API — on a
//! raster surface they rasterize immediately; on the Ganesh surface they
//! record into the deferred display list that `submit` flushes). `submit`
//! is flush + readback/GPU timestamps.
//!
//! GPU time for `skia-vulkan` comes from an adapter-owned `VkQueryPool`:
//! after a full `vkDeviceWaitIdle` drain, a standalone command buffer
//! writes a timestamp on the same `VkQueue` Skia uses, then Skia's
//! flush+submit runs, then a second timestamp submission and readback.
//! `skia-metal` instead brackets Graphite's insert+submit with empty
//! `MTLCommandBuffer` markers on the same serial queue, each preceded by
//! a full drain (`commit` + `waitUntilCompleted`) — Graphite exposes no
//! GPU timestamp source through `skia-safe` (`GpuStats` is Ganesh-only),
//! and command buffers on one Metal queue may overlap in execution on
//! Apple GPUs, so an undrained marker could bracket an empty interval.
//! This serializes CPU and GPU for the measured frame — a synchronous
//! probe, not a pipelined frame rate.
//!
//! Variable fonts: scene `normalized_coords` are post-avar normalized
//! values; Skia wants design (user) coordinates, so the adapter inverts
//! the avar1 per-axis segment maps and then unnormalizes through fvar.
//! For `avar` version 2 fonts the var-store warps cannot be inverted
//! pointwise — only the segment maps are applied, an approximation
//! documented in `sk_font`.

use std::collections::BTreeSet;
use std::time::Instant;

use cherenkov_oracle::{F32Image, color as oc};
use cherenkov_scene::{
    BlendMode as SBlend, Draw, Extend, Feature, FillRule, GlyphRun, Item, Layer, Paint as SPaint,
    ResourceHash, Sampling as SSampling, Scene, StrokeStyle,
};
use kurbo::{Affine, BezPath, PathEl};
use read_fonts::TableProvider;
use read_fonts::tables::avar::AxisValueMap;
use skia_safe::{
    AlphaType, Canvas, ClipOp, Color4f, ColorSpace, ColorType, Data, Font, FontArguments, FontMgr,
    FourByteTag, GlyphId, Image, ImageInfo, Matrix, Paint, Path, PathBuilder, PathEffect,
    PathFillType, Point, Rect, SamplingOptions, Surface, TileMode,
    canvas::{SaveLayerRec, SrcRectConstraint},
    font_arguments::{VariationPosition, variation_position},
    gradient::{
        Colors as GradColors, Gradient, Interpolation, interpolation as grad_interpolation,
        shaders as grad_shaders,
    },
    image_filters, named_primaries, named_transfer_fn,
    paint::{Cap, Join, Style},
    sampling_options::{FilterMode, MipmapMode},
    surfaces,
};

#[cfg(any(target_os = "linux", target_os = "android"))]
use crate::GpuSample;
use crate::convert::{self, Blobs};
use crate::memory::{AdapterMemory, Reading};
#[cfg(all(feature = "skia", any(target_os = "linux", target_os = "android")))]
use crate::memory::{ash_vk_memory_budget, skia_budget};
use crate::{BenchError, Counters, DeviceInfo, EncodeInput, Engine, EngineInfo, Submit};

/// Features the Skia adapter executes faithfully on its `RGBAF16`
/// linear-P3 route.
///
/// All scene features: [`Extend::None`] via `TileMode::Decal`,
/// [`Feature::Shadow`] for every shape via a blur image filter, real
/// HDR/wide-gamut output, and every gradient interpolation space
/// `SkGradient_Interpolation_ColorSpace` names — `linear-p3` is the one
/// absence and is reported unsupported.
#[must_use]
pub fn skia_features() -> Vec<Feature> {
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
        Feature::HdrColor,
        Feature::WideGamut,
        // Skia maps `Extend::None` to `SkTileMode::kDecal`.
        Feature::ExtendNone,
        Feature::InterpolationSpace(cherenkov_scene::ColorSpace::Srgb),
        Feature::InterpolationSpace(cherenkov_scene::ColorSpace::LinearSrgb),
        Feature::InterpolationSpace(cherenkov_scene::ColorSpace::DisplayP3),
        Feature::InterpolationSpace(cherenkov_scene::ColorSpace::Rec2020),
    ];
    for m in SBlend::ALL {
        v.push(Feature::Blend(m));
    }
    v
}

/// The upstream API the Skia adapter lacks for a declared feature — the
/// `missing_api` recorded on the report.
#[must_use]
pub const fn skia_missing_api(f: &Feature) -> Option<&'static str> {
    match f {
        Feature::InterpolationSpace(_) => {
            Some("SkGradient_Interpolation_ColorSpace has no linear-P3 value (Skia m143)")
        }
        _ => None,
    }
}

/// The surface colour space: linear Display P3 via CICP (P3-D65 primaries
/// id 12, linear transfer id 8) — the suite working space.
///
/// # Errors
/// [`BenchError::Gpu`] when Skia rejects the standard CICP combo.
fn p3_cs(engine: &'static str) -> Result<ColorSpace, BenchError> {
    ColorSpace::new_cicp(
        named_primaries::CicpId::SMPTE_EG_432_1,
        named_transfer_fn::CicpId::Linear,
    )
    .ok_or_else(|| {
        BenchError::Gpu(format!(
            "{engine}: SkColorSpace::MakeCICP(12, 8) returned null"
        ))
    })
}

/// One recorded canvas operation.
enum Cmd {
    /// `canvas.draw_paint(paint)` — paints over the whole clip; a paint
    /// carries a `Color4f` + colour space, unlike `clear(Color)` which
    /// quantizes to rgba8.
    DrawPaint(Paint),
    /// `canvas.save()`.
    Save,
    /// `canvas.restore()`.
    Restore,
    /// `canvas.concat(m)`.
    Concat(Matrix),
    /// `canvas.clip_path(p, Intersect, aa)`.
    Clip(Path),
    /// `canvas.save_layer(paint)` (blend + alpha).
    SaveLayer(Paint),
    /// `canvas.draw_path(p, paint)` (fill or stroke paint).
    Draw(Path, Paint),
    /// `canvas.draw_image_rect` with sampling.
    Image(Image, Rect, SamplingOptions, Paint),
    /// `canvas.draw_glyphs_at`.
    Glyphs(Font, Vec<GlyphId>, Vec<Point>, Paint),
}

/// Converts the scene to the adapter's pre-resolved form — a `Cmd` list
/// of Skia-native objects (paths, paints with live shaders, `Font`s,
/// raster `Image`s). Called once in `Engine::prepare`; the per-frame
/// `encode` only replays it on the canvas.
fn build_cmds(
    scene: &Scene,
    blobs: &Blobs,
    engine: &'static str,
    counters: &mut Counters,
) -> Result<Vec<Cmd>, BenchError> {
    let mut clear = Paint::default();
    clear.set_color4f(p3_linear4f(&scene.clear), &p3_cs(engine)?);
    let mut cmds = vec![Cmd::DrawPaint(clear)];
    encode_layer(&mut cmds, blobs, &scene.root, engine, counters)?;
    Ok(cmds)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "Skia APIs take f32; scene geometry fits"
)]
fn encode_layer(
    cmds: &mut Vec<Cmd>,
    blobs: &Blobs,
    layer: &Layer,
    engine: &'static str,
    counters: &mut Counters,
) -> Result<(), BenchError> {
    counters.layers += 1;
    cmds.push(Cmd::Save);
    cmds.push(Cmd::Concat(sk_matrix(layer.transform)));
    let grouped = layer.clip.is_some() || layer.blend != SBlend::Normal || layer.opacity < 1.0;
    if grouped {
        if let Some(clip) = &layer.clip {
            cmds.push(Cmd::Clip(sk_path(
                &convert::shape_path(clip),
                FillRule::NonZero,
            )));
        }
        let mut lp = Paint::default();
        lp.set_blend_mode(blend(layer.blend));
        lp.set_alpha_f(layer.opacity.clamp(0.0, 1.0) as f32);
        cmds.push(Cmd::SaveLayer(lp));
    }
    // Content and children draw translated by -scroll_offset inside the
    // clip; `motion` is unsupported (not in `skia_features`).
    if layer.scroll_offset != kurbo::Vec2::ZERO {
        cmds.push(Cmd::Concat(sk_matrix(kurbo::Affine::translate((
            -layer.scroll_offset.x,
            -layer.scroll_offset.y,
        )))));
    }
    for item in &layer.items {
        match item {
            Item::Layer(l) => encode_layer(cmds, blobs, l, engine, counters)?,
            Item::Draw(d) => {
                counters.draw_commands += 1;
                encode_draw(cmds, blobs, d, engine)?;
            }
            // Unreachable: scenes declaring `blend-space` fail the feature
            // check; a group with no declared feature needs no support.
            Item::Group(_) => {
                return Err(BenchError::Unsupported {
                    engine,
                    feature: Feature::BlendSpace(cherenkov_scene::BlendSpace::SrgbEncoded),
                    api: Some("skia: scoped group compositing"),
                });
            }
        }
    }
    if grouped {
        cmds.push(Cmd::Restore);
    }
    cmds.push(Cmd::Restore);
    Ok(())
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "Skia APIs take f32; scene geometry fits"
)]
fn encode_draw(
    cmds: &mut Vec<Cmd>,
    blobs: &Blobs,
    draw: &Draw,
    engine: &'static str,
) -> Result<(), BenchError> {
    match draw {
        Draw::Fill { shape, rule, paint } => {
            cmds.push(Cmd::Draw(
                sk_path(&convert::shape_path(shape), *rule),
                mk_paint(engine, paint, blobs)?,
            ));
        }
        Draw::Stroke {
            shape,
            stroke,
            paint,
        } => {
            let mut p = mk_paint(engine, paint, blobs)?;
            stroke_paint(engine, stroke, &mut p)?;
            cmds.push(Cmd::Draw(
                sk_path(&convert::shape_path(shape), FillRule::NonZero),
                p,
            ));
        }
        Draw::Image {
            image,
            dst,
            sampling,
            ..
        } => {
            let (w, h, rgba) = convert::decode_png(blob(blobs, *image)?)?;
            let img = sk_image(w, h, &rgba);
            let mut p = Paint::default();
            p.set_anti_alias(true);
            cmds.push(Cmd::Image(img, sk_rect(*dst), sampling_opts(*sampling), p));
        }
        Draw::Glyphs(run) => {
            let font = sk_font(engine, blobs, run)?;
            let ids: Vec<GlyphId> = run
                .glyphs
                .iter()
                .map(|g| u16::try_from(g.id))
                .collect::<Result<_, _>>()
                .map_err(|_| BenchError::Engine("skia: glyph id does not fit u16".into()))?;
            let pos = run.glyphs.iter().map(|g| Point::new(g.x, g.y)).collect();
            cmds.push(Cmd::Glyphs(
                font,
                ids,
                pos,
                mk_paint(engine, &run.paint, blobs)?,
            ));
        }
        Draw::Shadow {
            shape,
            blur_sigma,
            offset,
            color,
        } => {
            let mut p = Paint::default();
            p.set_anti_alias(true);
            p.set_color4f(p3_linear4f(color), &p3_cs(engine)?);
            p.set_image_filter(image_filters::blur(
                (*blur_sigma as f32, *blur_sigma as f32),
                TileMode::Decal,
                None,
                image_filters::CropRect::default(),
            ));
            let path = sk_path(&convert::shape_path(shape), FillRule::NonZero)
                .with_offset((offset[0] as f32, offset[1] as f32));
            cmds.push(Cmd::Draw(path, p));
        }
    }
    Ok(())
}

#[expect(clippy::cast_precision_loss, reason = "image pixel dims fit f32")]
fn replay(canvas: &Canvas, cmds: &[Cmd]) {
    for cmd in cmds {
        match cmd {
            Cmd::DrawPaint(p) => {
                canvas.draw_paint(p);
            }
            Cmd::Save => {
                canvas.save();
            }
            Cmd::Restore => {
                canvas.restore();
            }
            Cmd::Concat(m) => {
                canvas.concat(m);
            }
            Cmd::Clip(p) => {
                canvas.clip_path(p, ClipOp::Intersect, true);
            }
            Cmd::SaveLayer(p) => {
                canvas.save_layer(&SaveLayerRec::default().paint(p));
            }
            Cmd::Draw(p, paint) => {
                canvas.draw_path(p, paint);
            }
            Cmd::Image(img, dst, sam, paint) => {
                let src = Rect::from_xywh(0.0, 0.0, img.width() as f32, img.height() as f32);
                canvas.draw_image_rect_with_sampling_options(
                    img,
                    Some((&src, SrcRectConstraint::Fast)),
                    dst,
                    *sam,
                    paint,
                );
            }
            Cmd::Glyphs(font, ids, pos, paint) => {
                canvas.draw_glyphs_at(
                    ids.as_slice(),
                    pos.as_slice(),
                    Point::new(0.0, 0.0),
                    font,
                    paint,
                );
            }
        }
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "Skia APIs take f32; scene geometry fits"
)]
fn mk_paint(engine: &'static str, paint: &SPaint, blobs: &Blobs) -> Result<Paint, BenchError> {
    let mut p = Paint::default();
    p.set_anti_alias(true);
    let p3 = p3_cs(engine)?;
    match paint {
        SPaint::Transformed { .. } => {
            return Err(BenchError::Unsupported {
                engine,
                feature: Feature::PaintTransform,
                api: Some("independent paint transform adapter"),
            });
        }
        SPaint::Solid(c) => {
            p.set_color4f(p3_linear4f(c), &p3);
        }
        SPaint::Linear(g) => {
            let (colors, pos, tm, interp) =
                grad_parts(engine, &g.stops, g.extend, g.interpolation)?;
            let grad = Gradient::new(GradColors::new(&colors, Some(&pos), tm, p3), interp);
            p.set_shader(grad_shaders::linear_gradient(
                (sk_pt(g.start), sk_pt(g.end)),
                &grad,
                None,
            ));
        }
        SPaint::Radial(g) => {
            let (colors, pos, tm, interp) =
                grad_parts(engine, &g.stops, g.extend, g.interpolation)?;
            let grad = Gradient::new(GradColors::new(&colors, Some(&pos), tm, p3), interp);
            p.set_shader(grad_shaders::two_point_conical_gradient(
                (sk_pt(g.center0), g.r0 as f32),
                (sk_pt(g.center1), g.r1 as f32),
                &grad,
                None,
            ));
        }
        SPaint::Sweep(g) => {
            let (colors, pos, tm, interp) =
                grad_parts(engine, &g.stops, g.extend, g.interpolation)?;
            let grad = Gradient::new(GradColors::new(&colors, Some(&pos), tm, p3), interp);
            p.set_shader(grad_shaders::sweep_gradient(
                sk_pt(g.center),
                (
                    (g.start_angle as f32).to_degrees(),
                    (g.end_angle as f32).to_degrees(),
                ),
                &grad,
                None,
            ));
        }
        SPaint::Mesh(_) => {
            return Err(BenchError::Unsupported {
                engine,
                feature: Feature::MeshGradient,
                api: Some("bilinear mesh paint"),
            });
        }
        SPaint::Image(ip) => {
            let (w, h, rgba) = convert::decode_png(blob(blobs, ip.image)?)?;
            let img = sk_image(w, h, &rgba);
            let tm = (tile(ip.extend_x), tile(ip.extend_y));
            let m = sk_matrix(ip.transform);
            p.set_shader(img.to_shader(Some(tm), sampling_opts(ip.sampling), &m));
        }
    }
    Ok(p)
}

/// Maps a scene interpolation space onto Skia's gradient interpolation
/// colour spaces; `linear-p3` has no value and is reported unsupported.
const fn interpolation_space(
    engine: &'static str,
    space: cherenkov_scene::ColorSpace,
) -> Result<grad_interpolation::ColorSpace, BenchError> {
    use cherenkov_scene::ColorSpace as S;
    Ok(match space {
        S::Srgb => grad_interpolation::ColorSpace::SRGB,
        S::LinearSrgb => grad_interpolation::ColorSpace::SRGBLinear,
        S::DisplayP3 => grad_interpolation::ColorSpace::DisplayP3,
        S::Rec2020 => grad_interpolation::ColorSpace::Rec2020,
        S::LinearP3 => {
            return Err(BenchError::Unsupported {
                engine,
                feature: Feature::InterpolationSpace(space),
                api: skia_missing_api(&Feature::InterpolationSpace(space)),
            });
        }
    })
}

/// Converts scene gradient stops to owned Skia inputs. Stop colours are
/// expressed in the surface's linear-P3 space; Skia interpolates between
/// them in the declared interpolation space.
fn grad_parts(
    engine: &'static str,
    stops: &[cherenkov_scene::GradientStop],
    extend: Extend,
    interpolation: cherenkov_scene::ColorSpace,
) -> Result<(Vec<Color4f>, Vec<f32>, TileMode, Interpolation), BenchError> {
    if stops.is_empty() {
        return Err(BenchError::Engine(format!(
            "{engine}: gradient with no stops"
        )));
    }
    let colors: Vec<Color4f> = stops.iter().map(|s| p3_linear4f(&s.color)).collect();
    let pos: Vec<f32> = stops.iter().map(|s| s.offset).collect();
    let interp = Interpolation {
        color_space: interpolation_space(engine, interpolation)?,
        ..Interpolation::default()
    };
    Ok((colors, pos, tile(extend), interp))
}

/// Maps the scene colour to straight `Color4f` in linear P3 — the surface's
/// own colour space, so values pass through unclamped and HDR/wide-gamut
/// content survives (unlike the old rgba8 sRGB route which clamped).
#[expect(clippy::many_single_char_names, reason = "r/g/b/a channel names")]
#[expect(
    clippy::cast_possible_truncation,
    reason = "Skia Color4f takes f32; linear sRGB values fit"
)]
fn p3_linear4f(c: &cherenkov_scene::Color) -> Color4f {
    let [r, g, b, a] = oc::to_working(c);
    let a = a.clamp(0.0, 1.0);
    let [r, g, b] = if a > 0.0 {
        [r / a, g / a, b / a]
    } else {
        [0.0; 3]
    };
    Color4f::new(r as f32, g as f32, b as f32, a as f32)
}

/// Inverts one avar1 segment map. `axis_value_maps` maps pre-avar
/// normalized coordinates (`from_coordinate`) to post-avar ones
/// (`to_coordinate`); it is piecewise linear and non-decreasing, so the
/// piecewise inverse is defined except on degenerate (zero-length)
/// segments.
fn avar_invert(maps: &[AxisValueMap], post: f64) -> f64 {
    let from = |m: &AxisValueMap| m.from_coordinate().to_f64();
    let to = |m: &AxisValueMap| m.to_coordinate().to_f64();
    if maps.len() < 2 {
        return maps.first().map_or(post, |m| post - to(m) + from(m));
    }
    if post <= to(&maps[0]) {
        return from(&maps[0]);
    }
    for w in maps.windows(2) {
        let (t0, t1) = (to(&w[0]), to(&w[1]));
        if post <= t1 {
            let f = if t1 > t0 {
                (post - t0) / (t1 - t0)
            } else {
                0.0
            };
            return from(&w[0]).mul_add(1.0 - f, from(&w[1]) * f);
        }
    }
    from(maps.last().expect("len >= 2"))
}

/// Converts the scene's post-avar normalized variation coordinates to the
/// design (user) coordinates `set_variation_design_position` expects:
/// invert the font's avar1 per-axis segment map, then unnormalize through
/// the fvar axis ranges. `avar` version 2 adds var-store warps that cannot
/// be inverted pointwise — for those fonts only the segment maps are
/// inverted, an approximation recorded in the adapter's notes.
#[expect(
    clippy::cast_possible_truncation,
    reason = "fvar design values fit f32"
)]
fn design_coords(
    engine: &'static str,
    bytes: &[u8],
    font_index: u32,
    coords: &[cherenkov_scene::NormalizedCoord],
) -> Result<Vec<variation_position::Coordinate>, BenchError> {
    let font = skrifa::FontRef::from_index(bytes, font_index)
        .map_err(|e| BenchError::Engine(format!("{engine}: font not parsed: {e}")))?;
    let axes = skrifa::MetadataProvider::axes(&font);
    let avar = font.avar().ok().filter(|a| a.version().major == 1);
    // The iterator yields one entry per axis in order; collect the
    // Results so element `i` always addresses axis `i` — skipping a
    // failed map would misalign the indices — and propagate a failed
    // map instead of falling back to a linear warp.
    let seg_maps: Vec<read_fonts::tables::avar::SegmentMaps<'_>> = match avar {
        Some(a) => a
            .axis_segment_maps()
            .iter()
            .collect::<Result<_, _>>()
            .map_err(|e| BenchError::Engine(format!("{engine}: avar segment map failed: {e}")))?,
        None => Vec::new(),
    };
    coords
        .iter()
        .map(|c| {
            let tag = tag4(engine, &c.tag)?;
            let font_axis = axes.get_by_tag(skrifa::Tag::from_u32(tag)).ok_or_else(|| {
                BenchError::Engine(format!("{engine}: font has no {:?} variation axis", c.tag))
            })?;
            let n = f64::from(c.value);
            let pre = seg_maps
                .get(font_axis.index())
                .map_or(n, |m| avar_invert(m.axis_value_maps(), n));
            let (min, def, max) = (
                f64::from(font_axis.min_value()),
                f64::from(font_axis.default_value()),
                f64::from(font_axis.max_value()),
            );
            let design = if pre >= 0.0 {
                pre.mul_add(max - def, def)
            } else {
                pre.mul_add(def - min, def)
            };
            Ok(variation_position::Coordinate {
                axis: FourByteTag::from(tag),
                value: design as f32,
            })
        })
        .collect()
}

fn sk_font(engine: &'static str, blobs: &Blobs, run: &GlyphRun) -> Result<Font, BenchError> {
    let bytes = blob(blobs, run.font)?;
    let data = Data::new_copy(bytes.as_slice());
    let fm = FontMgr::new();
    let tf = fm.new_from_data(data, run.font_index).ok_or({
        // `newFromData` delegates to the platform font service; on some
        // platforms it rejects bitmap-only fonts that carry no outline
        // tables, which is an unsupported case for this adapter, not an
        // engine error.
        BenchError::Unsupported {
            engine,
            feature: Feature::Glyphs,
            api: Some("platform typeface creation requires glyph outlines"),
        }
    })?;
    let tf = if run.normalized_coords.is_empty() {
        tf
    } else {
        let coords = design_coords(engine, bytes, run.font_index, &run.normalized_coords)?;
        let args = FontArguments::new().set_variation_design_position(VariationPosition {
            coordinates: &coords,
        });
        tf.clone_with_arguments(&args)
            .ok_or_else(|| BenchError::Engine(format!("{engine}: variation clone failed")))?
    };
    Ok(Font::from_typeface(tf, run.size))
}

fn tag4(engine: &'static str, tag: &str) -> Result<u32, BenchError> {
    let b = tag.as_bytes();
    if b.len() != 4 {
        return Err(BenchError::Engine(format!(
            "{engine}: bad axis tag {tag:?}"
        )));
    }
    Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "Skia APIs take f32; stroke geometry fits"
)]
fn stroke_paint(
    engine: &'static str,
    style: &StrokeStyle,
    p: &mut Paint,
) -> Result<(), BenchError> {
    p.set_style(Style::Stroke);
    p.set_stroke_width(style.width as f32);
    p.set_stroke_miter(style.miter_limit as f32);
    p.set_stroke_join(match style.join {
        kurbo::Join::Miter => Join::Miter,
        kurbo::Join::Round => Join::Round,
        kurbo::Join::Bevel => Join::Bevel,
    });
    if style.start_cap != style.end_cap {
        return Err(BenchError::Unsupported {
            engine,
            feature: Feature::Stroke,
            api: Some("SkPaint::setStrokeCap sets a single cap for both ends of a stroke"),
        });
    }
    p.set_stroke_cap(match style.start_cap {
        kurbo::Cap::Butt => Cap::Butt,
        kurbo::Cap::Round => Cap::Round,
        kurbo::Cap::Square => Cap::Square,
    });
    if !style.dash_pattern.is_empty() {
        let intervals: Vec<f32> = style.dash_pattern.iter().map(|v| *v as f32).collect();
        p.set_path_effect(PathEffect::dash(&intervals, style.dash_offset as f32));
    }
    Ok(())
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "Skia paths take f32; scene geometry fits"
)]
fn sk_path(bez: &BezPath, rule: FillRule) -> Path {
    let mut b = PathBuilder::new_with_fill_type(match rule {
        FillRule::NonZero => PathFillType::Winding,
        FillRule::EvenOdd => PathFillType::EvenOdd,
    });
    for el in bez.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                b.move_to((p.x as f32, p.y as f32));
            }
            PathEl::LineTo(p) => {
                b.line_to((p.x as f32, p.y as f32));
            }
            PathEl::QuadTo(p1, p2) => {
                b.quad_to((p1.x as f32, p1.y as f32), (p2.x as f32, p2.y as f32));
            }
            PathEl::CurveTo(p1, p2, p3) => {
                b.cubic_to(
                    (p1.x as f32, p1.y as f32),
                    (p2.x as f32, p2.y as f32),
                    (p3.x as f32, p3.y as f32),
                );
            }
            PathEl::ClosePath => {
                b.close();
            }
        }
    }
    b.detach()
}

fn sk_image(w: u32, h: u32, rgba: &[u8]) -> Image {
    let info = ImageInfo::new(
        (
            i32::try_from(w).expect("image dims fit i32"),
            i32::try_from(h).expect("image dims fit i32"),
        ),
        ColorType::RGBA8888,
        AlphaType::Unpremul,
        ColorSpace::new_srgb(),
    );
    skia_safe::images::raster_from_data(&info, Data::new_copy(rgba), w as usize * 4)
        .expect("skia raster image")
}

#[expect(clippy::cast_possible_truncation, reason = "Skia matrices take f32")]
#[allow(clippy::many_single_char_names)] // matrix coefficient names
fn sk_matrix(t: Affine) -> Matrix {
    let [a, b, c, d, e, f] = t.as_coeffs();
    Matrix::new_all(
        a as f32, c as f32, e as f32, b as f32, d as f32, f as f32, 0.0, 0.0, 1.0,
    )
}

#[expect(clippy::cast_possible_truncation, reason = "Skia points take f32")]
const fn sk_pt(p: kurbo::Point) -> Point {
    Point::new(p.x as f32, p.y as f32)
}

#[expect(clippy::cast_possible_truncation, reason = "Skia rects take f32")]
fn sk_rect(r: kurbo::Rect) -> Rect {
    Rect::from_ltrb(r.x0 as f32, r.y0 as f32, r.x1 as f32, r.y1 as f32)
}

const fn tile(e: Extend) -> TileMode {
    match e {
        Extend::Pad => TileMode::Clamp,
        Extend::Repeat => TileMode::Repeat,
        Extend::Reflect => TileMode::Mirror,
        Extend::None => TileMode::Decal,
    }
}

fn sampling_opts(s: SSampling) -> SamplingOptions {
    match s {
        SSampling::Nearest => SamplingOptions::new(FilterMode::Nearest, MipmapMode::None),
        SSampling::Bilinear => SamplingOptions::new(FilterMode::Linear, MipmapMode::None),
    }
}

const fn blend(m: SBlend) -> skia_safe::BlendMode {
    use skia_safe::BlendMode as B;
    match m {
        SBlend::Normal => B::SrcOver,
        SBlend::Multiply => B::Multiply,
        SBlend::Screen => B::Screen,
        SBlend::Overlay => B::Overlay,
        SBlend::Darken => B::Darken,
        SBlend::Lighten => B::Lighten,
        SBlend::ColorDodge => B::ColorDodge,
        SBlend::ColorBurn => B::ColorBurn,
        SBlend::HardLight => B::HardLight,
        SBlend::SoftLight => B::SoftLight,
        SBlend::Difference => B::Difference,
        SBlend::Exclusion => B::Exclusion,
        SBlend::Hue => B::Hue,
        SBlend::Saturation => B::Saturation,
        SBlend::Color => B::Color,
        SBlend::Luminosity => B::Luminosity,
        // Skia's non-Mix modes cover the COLRv1 compositing operators.
        SBlend::Clear => B::Clear,
        SBlend::Src => B::Src,
        SBlend::Dst => B::Dst,
        SBlend::DestOver => B::DstOver,
        SBlend::SrcIn => B::SrcIn,
        SBlend::DestIn => B::DstIn,
        SBlend::SrcOut => B::SrcOut,
        SBlend::DestOut => B::DstOut,
        SBlend::SrcAtop => B::SrcATop,
        SBlend::DestAtop => B::DstATop,
        SBlend::Xor => B::Xor,
        SBlend::PlusLighter => B::Plus,
    }
}

/// Reads the surface as premultiplied `f16` linear-P3 pixels and decodes
/// to the suite `f32` working image — HDR values survive the round trip.
fn read_surface(
    surface: &mut Surface,
    engine: &'static str,
    w: u32,
    h: u32,
) -> Result<F32Image, BenchError> {
    let info = ImageInfo::new(
        (
            i32::try_from(w).expect("surface dims fit i32"),
            i32::try_from(h).expect("surface dims fit i32"),
        ),
        ColorType::RGBAF16,
        AlphaType::Premul,
        p3_cs(engine)?,
    );
    let mut buf = vec![0u8; w as usize * h as usize * 8];
    if !surface.read_pixels(&info, &mut buf, w as usize * 8, (0, 0)) {
        return Err(BenchError::Gpu("skia read_pixels failed".into()));
    }
    let pixels = buf
        .as_chunks::<8>()
        .0
        .iter()
        .map(|q| {
            [
                convert::f16_to_f32(u16::from_le_bytes([q[0], q[1]])),
                convert::f16_to_f32(u16::from_le_bytes([q[2], q[3]])),
                convert::f16_to_f32(u16::from_le_bytes([q[4], q[5]])),
                convert::f16_to_f32(u16::from_le_bytes([q[6], q[7]])),
            ]
        })
        .collect();
    Ok(F32Image {
        width: w,
        height: h,
        pixels,
    })
}

fn blob(blobs: &Blobs, hash: ResourceHash) -> Result<&Vec<u8>, BenchError> {
    blobs
        .get(&hash)
        .ok_or_else(|| cherenkov_scene::SceneError::MissingResource(hash).into())
}

/// `skia-cpu`: Ganesh/raster CPU surface adapter.
pub struct SkiaCpu {
    info: EngineInfo,
    /// Prepared scene state: the surface and the resolved `Cmd` list.
    prepared: Option<SkiaPrepared>,
    size: (u32, u32),
    counters: Counters,
}

impl std::fmt::Debug for SkiaCpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkiaCpu")
            .field("info", &self.info)
            .field("size", &self.size)
            .field("counters", &self.counters)
            .finish_non_exhaustive()
    }
}

/// The pre-resolved scene state `prepare` builds once per scene: the
/// render-target surface an app would reuse, plus the `Cmd` list holding
/// Skia-native resources (fonts, images, shaders) resolved once.
struct SkiaPrepared {
    surface: Surface,
    cmds: Vec<Cmd>,
}

impl SkiaCpu {
    /// Adapter key.
    pub const NAME: &'static str = "skia-cpu";

    /// Creates the adapter.
    #[must_use]
    pub fn new() -> Self {
        Self {
            info: EngineInfo {
                name: Self::NAME,
                engine_crate: "skia-safe",
                crate_version: env!("DEP_SKIA_SAFE_VERSION"),
                source_rev: option_env!("DEP_SKIA_SAFE_SOURCE_REV").map(String::from),
                output_format:
                    "RGBAF16 premultiplied raster surface (linear Display P3, CICP 12/8)".into(),
                precision: "Skia blends in f32 premultiplied; output is f16 (≈11-bit mantissa)",
                route: "cpu-raster",
                color_note: "scene colours converted working-space→linear-P3 f32 unclamped; \
                             readback f16→f32",
                encode_scope: "replays pre-resolved `Canvas` draw calls — on a raster surface \
                               the calls rasterize as they record",
            },
            prepared: None,
            size: (0, 0),
            counters: Counters::default(),
        }
    }
}

impl Default for SkiaCpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine for SkiaCpu {
    fn info(&self) -> &EngineInfo {
        &self.info
    }

    fn supported(&self) -> BTreeSet<Feature> {
        skia_features().into_iter().collect()
    }

    fn prepare(&mut self, input: &EncodeInput<'_>) -> Result<(), BenchError> {
        convert::check_features(Self::NAME, input.scene, &skia_features(), skia_missing_api)?;
        self.counters = Counters::default();
        let (w, h) = (input.scene.width, input.scene.height);
        let info = ImageInfo::new(
            (
                i32::try_from(w).expect("surface dims fit i32"),
                i32::try_from(h).expect("surface dims fit i32"),
            ),
            ColorType::RGBAF16,
            AlphaType::Premul,
            p3_cs(Self::NAME)?,
        );
        let surface = surfaces::raster(&info, None, None)
            .ok_or_else(|| BenchError::Gpu("skia raster surface".into()))?;
        self.size = (w, h);
        self.prepared = Some(SkiaPrepared {
            surface,
            cmds: build_cmds(input.scene, input.blobs, Self::NAME, &mut self.counters)?,
        });
        Ok(())
    }

    fn encode(&mut self, _input: &EncodeInput<'_>) -> Result<(), BenchError> {
        let prepared = self
            .prepared
            .as_mut()
            .ok_or_else(|| BenchError::Engine("skia-cpu: encode before prepare".into()))?;
        replay(prepared.surface.canvas(), &prepared.cmds);
        Ok(())
    }

    fn submit(&mut self, _frame: u64, readback: bool) -> Result<Submit, BenchError> {
        let render_at = Instant::now();
        let prepared = self
            .prepared
            .as_mut()
            .ok_or_else(|| BenchError::Engine("skia-cpu: submit before prepare".into()))?;
        // Rasterization happened inside `encode`'s canvas calls on the
        // raster surface; the submit itself only snapshots and reads.
        let render_seconds = render_at.elapsed().as_secs_f64();
        let readback_at = Instant::now();
        let image = if readback {
            Some(read_surface(
                &mut prepared.surface,
                Self::NAME,
                self.size.0,
                self.size.1,
            )?)
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

    fn counters(&self) -> Counters {
        self.counters.clone()
    }

    fn device(&self) -> DeviceInfo {
        DeviceInfo {
            adapter: Some("Skia raster surface".into()),
            backend: Some("cpu".into()),
            cpu: crate::cpu_model(),
            thermal_celsius: crate::thermal_celsius(),
            ..DeviceInfo::default()
        }
    }

    fn memory(&self) -> AdapterMemory {
        AdapterMemory {
            engine: Reading::unavailable("Skia raster does not expose engine memory"),
            wgpu_allocator: Reading::unavailable("Skia raster has no wgpu allocator"),
            skia_budgeted: Reading::unavailable("Skia raster has no budgeted GPU resources"),
            vk_memory_budget: Reading::unavailable("Skia raster has no Vulkan device"),
        }
    }
}

/// `skia-vulkan`: Ganesh Vulkan adapter (Linux and Android only).
#[cfg(any(target_os = "linux", target_os = "android"))]
pub struct SkiaVk {
    info: EngineInfo,
    dctx: skia_safe::gpu::DirectContext,
    _bctx: skia_safe::gpu::vk::BackendContext<'static>,
    vk_instance: &'static ash::Instance,
    vk_physical_device: ash::vk::PhysicalDevice,
    /// Raw device handle for our own timestamp submissions.
    vk_dev: &'static ash::Device,
    /// The queue Skia submits on — the timestamps must run on it too.
    vk_queue: ash::vk::Queue,
    /// Timestamp pool + command buffers (None when the queue family
    /// reports `timestamp_valid_bits == 0`).
    vk_ts: Option<VkTs>,
    /// Prepared scene state: the Ganesh surface and resolved `Cmd` list.
    prepared: Option<SkiaPrepared>,
    size: (u32, u32),
    counters: Counters,
    device_name: String,
}

/// The adapter fields Vulkan and Skia handle types do not format.
#[cfg(any(target_os = "linux", target_os = "android"))]
impl std::fmt::Debug for SkiaVk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkiaVk")
            .field("info", &self.info)
            .field("size", &self.size)
            .field("counters", &self.counters)
            .field("device_name", &self.device_name)
            .finish_non_exhaustive()
    }
}

/// Adapter-owned timestamp resources: a `VkQueryPool` and command buffers
/// that write markers on Skia's queue around its flush+submit.
#[cfg(any(target_os = "linux", target_os = "android"))]
struct VkTs {
    query_pool: ash::vk::QueryPool,
    bufs: [ash::vk::CommandBuffer; 2],
    /// `VkPhysicalDeviceLimits::timestampPeriod` — nanoseconds per tick.
    period_ns: f64,
}

#[cfg(any(target_os = "linux", target_os = "android"))]
impl SkiaVk {
    /// Adapter key.
    pub const NAME: &'static str = "skia-vulkan";

    /// Creates the adapter, initializing a headless Vulkan device and a
    /// Ganesh [`DirectContext`][skia_safe::gpu::DirectContext].
    ///
    /// The `ash` entry/instance/device and the Skia get-proc resolver are
    /// leaked to `'static` so the [`BackendContext`]'s borrowed `GetProc`
    /// never dangles; the adapter lives for the whole process.
    ///
    /// # Errors
    /// [`BenchError::Gpu`] on any Vulkan or Skia failure.
    ///
    /// # Panics
    /// On internal setup invariants asserted with `expect`.
    #[expect(
        clippy::too_many_lines,
        reason = "one-pass Vulkan bring-up; splitting it adds plumbing, not clarity"
    )]
    pub fn new() -> Result<Self, BenchError> {
        use ash::vk::{self, Handle};
        use std::ffi::CStr;

        fn gerr(e: impl std::fmt::Display) -> BenchError {
            BenchError::Gpu(format!("vulkan: {e}"))
        }

        // SAFETY: loads the platform Vulkan loader; a missing driver is
        // reported as an error, never an invalid entry.
        let entry: &'static ash::Entry =
            Box::leak(Box::new(unsafe { ash::Entry::load() }.map_err(gerr)?));
        // SAFETY: `entry` is the live loader obtained above.
        let api = unsafe { entry.try_enumerate_instance_version() }
            .map_err(gerr)?
            .unwrap_or(vk::API_VERSION_1_1);
        let app_info = vk::ApplicationInfo::default().api_version(api);
        let instance_ci = vk::InstanceCreateInfo::default().application_info(&app_info);
        // SAFETY: entry is a live loader; the instance is leaked and outlives
        // every user of it.
        let instance: &'static ash::Instance = Box::leak(Box::new(
            unsafe { entry.create_instance(&instance_ci, None) }.map_err(gerr)?,
        ));
        // SAFETY: `instance` is the live instance leaked above.
        let pds = unsafe { instance.enumerate_physical_devices() }.map_err(gerr)?;
        let pd = *pds.first().ok_or_else(|| gerr("no physical device"))?;
        // SAFETY: `instance` and `pd` are the live handles from above.
        let qpos = unsafe { instance.get_physical_device_queue_family_properties(pd) }
            .iter()
            .position(|q| q.queue_flags.contains(vk::QueueFlags::GRAPHICS))
            .ok_or_else(|| gerr("no graphics queue"))?;
        let qfam = u32::try_from(qpos).expect("queue family index fits u32");
        let prios = [1.0f32];
        let qcis = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(qfam)
            .queue_priorities(&prios)];
        let dci = vk::DeviceCreateInfo::default().queue_create_infos(&qcis);
        // SAFETY: `pd`/`qfam` are valid; the device is leaked and outlives
        // every user of it.
        let device: &'static ash::Device = Box::leak(Box::new(
            unsafe { instance.create_device(pd, &dci, None) }.map_err(gerr)?,
        ));
        // SAFETY: queue 0 of the graphics family the device was created for.
        let queue = unsafe { device.get_device_queue(qfam, 0) };
        // SAFETY: `instance` and `pd` are the live handles from above.
        let props = unsafe { instance.get_physical_device_properties(pd) };
        // SAFETY: `device_name` is the fixed NUL-terminated array Vulkan
        // guarantees inside `props`, which outlives the borrow.
        let device_name = unsafe { CStr::from_ptr(props.device_name.as_ptr()) }
            .to_string_lossy()
            .into_owned();

        // Timestamp resources on the queue family Skia uses. `None` when
        // the family reports `timestamp_valid_bits == 0` — GPU time then
        // stays `null` rather than fabricated.
        // SAFETY: `instance` and `pd` are the live handles from above.
        let timestamps_valid = unsafe { instance.get_physical_device_queue_family_properties(pd) }
            .get(qfam as usize)
            .is_some_and(|q| q.timestamp_valid_bits > 0);
        let period_ns = f64::from(props.limits.timestamp_period);
        let vk_ts = if timestamps_valid {
            // SAFETY: `device` is the live leaked device and the
            // create-info is fully initialized.
            let qp = unsafe {
                device.create_query_pool(
                    &vk::QueryPoolCreateInfo::default()
                        .query_type(vk::QueryType::TIMESTAMP)
                        .query_count(2),
                    None,
                )
            }
            .map_err(gerr)?;
            // SAFETY: `device` is the live leaked device; the pool is
            // bound to the graphics queue family `qfam`.
            let cp = unsafe {
                device.create_command_pool(
                    &vk::CommandPoolCreateInfo::default()
                        .queue_family_index(qfam)
                        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                    None,
                )
            }
            .map_err(gerr)?;
            // SAFETY: `cp` is the live pool just created on `device`;
            // count 2 returns two buffers.
            let mut it = unsafe {
                device.allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(cp)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(2),
                )
            }
            .map_err(gerr)?;
            let b1 = it.pop().expect("two buffers allocated");
            let b0 = it.pop().expect("two buffers allocated");
            Some(VkTs {
                query_pool: qp,
                bufs: [b0, b1],
                period_ns,
            })
        } else {
            None
        };

        let resolver: &'static _ = Box::leak(Box::new(move |of: skia_safe::gpu::vk::GetProcOf| {
            // SAFETY: `inst`/`dev` are raw handles Skia derived from the
            // leaked `BackendContext` handles; a lookup miss maps to a
            // null function pointer, never a bad one.
            unsafe {
                match of {
                    skia_safe::gpu::vk::GetProcOf::Instance(inst, name) => entry
                        .get_instance_proc_addr(vk::Instance::from_raw(inst as u64), name)
                        .map_or(std::ptr::null(), |fp| fp as *const _),
                    skia_safe::gpu::vk::GetProcOf::Device(dev, name) => instance
                        .get_device_proc_addr(vk::Device::from_raw(dev as u64), name)
                        .map_or(std::ptr::null(), |fp| fp as *const _),
                }
            }
        }));

        // SAFETY: instance/device/queue and `resolver` are leaked `'static`,
        // satisfying BackendContext's "handles must outlive it" contract.
        let bctx = unsafe {
            skia_safe::gpu::vk::BackendContext::new_builder(
                instance.handle().as_raw() as skia_safe::gpu::vk::Instance,
                pd.as_raw() as skia_safe::gpu::vk::PhysicalDevice,
                device.handle().as_raw() as skia_safe::gpu::vk::Device,
                (queue.as_raw() as skia_safe::gpu::vk::Queue, qfam as usize),
                resolver,
                None,
            )
            .build()
        };
        let dctx = skia_safe::gpu::ganesh::vk::direct_contexts::make_vulkan(&bctx, None)
            .ok_or_else(|| gerr("skia make_vulkan returned None"))?;
        Ok(Self {
            info: EngineInfo {
                name: Self::NAME,
                engine_crate: "skia-safe",
                crate_version: env!("DEP_SKIA_SAFE_VERSION"),
                source_rev: option_env!("DEP_SKIA_SAFE_SOURCE_REV").map(String::from),
                output_format:
                    "Ganesh Vulkan render target, RGBAF16 premultiplied (linear Display P3)".into(),
                precision: "Skia blends in f32 premultiplied; output is f16 (≈11-bit mantissa)",
                route: "vulkan",
                color_note: "scene colours converted working-space→linear-P3 f32 unclamped; \
                             readback f16→f32",
                encode_scope: "replays pre-resolved `Canvas` draw calls — Ganesh records them \
                               into the deferred display list `submit` flushes",
            },
            dctx,
            _bctx: bctx,
            vk_instance: instance,
            vk_physical_device: pd,
            vk_dev: device,
            vk_queue: queue,
            vk_ts,
            prepared: None,
            size: (0, 0),
            counters: Counters::default(),
            device_name,
        })
    }

    /// Records a `vkCmdWriteTimestamp` (`index` into the pool) into
    /// `buf` and submits it on Skia's queue. `reset` clears the pool
    /// before the first marker of a pair.
    ///
    /// # Errors
    /// [`BenchError::Gpu`] on a Vulkan error.
    fn vk_stamp(
        &self,
        buf: ash::vk::CommandBuffer,
        index: u32,
        reset: bool,
    ) -> Result<(), BenchError> {
        use ash::vk;
        fn gerr(e: impl std::fmt::Display) -> BenchError {
            BenchError::Gpu(format!("vulkan timestamp: {e}"))
        }
        let Some(ts) = &self.vk_ts else {
            return Ok(());
        };
        let dev = self.vk_dev;
        // SAFETY: `buf` is one of the two buffers allocated from `ts`'s
        // command pool on `dev`, and `self.vk_queue` belongs to the pool's
        // queue family.
        unsafe {
            dev.reset_command_buffer(buf, vk::CommandBufferResetFlags::empty())
                .map_err(gerr)?;
            dev.begin_command_buffer(
                buf,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )
            .map_err(gerr)?;
            if reset {
                dev.cmd_reset_query_pool(buf, ts.query_pool, 0, 2);
            }
            dev.cmd_write_timestamp(
                buf,
                vk::PipelineStageFlags::ALL_COMMANDS,
                ts.query_pool,
                index,
            );
            dev.end_command_buffer(buf).map_err(gerr)?;
            let bufs = [buf];
            let si = vk::SubmitInfo::default().command_buffers(&bufs);
            dev.queue_submit(self.vk_queue, std::slice::from_ref(&si), vk::Fence::null())
                .map_err(gerr)?;
        }
        Ok(())
    }

    /// Reads the resolved timestamp pair, scaled by
    /// `VkPhysicalDeviceLimits::timestampPeriod`, in seconds.
    fn vk_read_seconds(&self) -> Result<Option<f64>, BenchError> {
        use ash::vk;
        fn gerr(e: impl std::fmt::Display) -> BenchError {
            BenchError::Gpu(format!("vulkan timestamp: {e}"))
        }
        let Some(ts) = &self.vk_ts else {
            return Ok(None);
        };
        let mut data = [0u64; 2];
        // SAFETY: `ts.query_pool` is live on `self.vk_dev`; `data` is a
        // `[u64; 2]` matching the pool's `query_count` of 2 with TYPE_64.
        unsafe {
            self.vk_dev.get_query_pool_results(
                ts.query_pool,
                0,
                &mut data,
                vk::QueryResultFlags::TYPE_64 | vk::QueryResultFlags::WAIT,
            )
        }
        .map_err(gerr)?;
        #[expect(
            clippy::cast_precision_loss,
            reason = "tick deltas are well below 2^53"
        )]
        let secs = (data[1] - data[0]) as f64 * ts.period_ns * 1e-9;
        Ok((data[1] > data[0]).then_some(secs))
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
impl Engine for SkiaVk {
    fn info(&self) -> &EngineInfo {
        &self.info
    }

    fn supported(&self) -> BTreeSet<Feature> {
        skia_features().into_iter().collect()
    }

    fn prepare(&mut self, input: &EncodeInput<'_>) -> Result<(), BenchError> {
        convert::check_features(Self::NAME, input.scene, &skia_features(), skia_missing_api)?;
        self.counters = Counters::default();
        let (w, h) = (input.scene.width, input.scene.height);
        let info = ImageInfo::new(
            (
                i32::try_from(w).expect("surface dims fit i32"),
                i32::try_from(h).expect("surface dims fit i32"),
            ),
            ColorType::RGBAF16,
            AlphaType::Premul,
            p3_cs(Self::NAME)?,
        );
        let surface = skia_safe::gpu::ganesh::surface_ganesh::render_target(
            &mut self.dctx,
            skia_safe::gpu::Budgeted::Yes,
            &info,
            None,
            skia_safe::gpu::SurfaceOrigin::TopLeft,
            None,
            false,
            false,
        )
        .ok_or_else(|| BenchError::Gpu("skia vulkan render target".into()))?;
        self.size = (w, h);
        self.prepared = Some(SkiaPrepared {
            surface,
            cmds: build_cmds(input.scene, input.blobs, Self::NAME, &mut self.counters)?,
        });
        Ok(())
    }

    fn encode(&mut self, _input: &EncodeInput<'_>) -> Result<(), BenchError> {
        let prepared = self
            .prepared
            .as_mut()
            .ok_or_else(|| BenchError::Engine("skia-vulkan: encode before prepare".into()))?;
        replay(prepared.surface.canvas(), &prepared.cmds);
        Ok(())
    }

    fn submit(&mut self, frame: u64, readback: bool) -> Result<Submit, BenchError> {
        if self.prepared.is_none() {
            return Err(BenchError::Engine(
                "skia-vulkan: submit before prepare".into(),
            ));
        }
        // GPU time: bracket Skia's flush+submit with our own
        // `vkCmdWriteTimestamp` submissions on the same queue, each
        // preceded by a full device drain so the marker cannot run
        // concurrently with the render (job-scheduled tiled GPUs would
        // otherwise execute it in parallel and bracket an empty
        // interval). This serializes CPU and GPU for the measured frame.
        let no_ts = self.vk_ts.is_none();
        if !no_ts {
            // SAFETY: `self.vk_dev` is the live leaked device; wait-idle
            // has no handle preconditions.
            unsafe { self.vk_dev.device_wait_idle() }
                .map_err(|e| BenchError::Gpu(format!("vulkan drain: {e}")))?;
            self.vk_stamp(self.vk_ts.as_ref().expect("checked").bufs[0], 0, true)?;
        }
        let render_at = Instant::now();
        self.dctx.flush_submit_and_sync_cpu();
        let gpu_seconds = if no_ts {
            None
        } else {
            // SAFETY: `self.vk_dev` is the live leaked device.
            unsafe { self.vk_dev.device_wait_idle() }
                .map_err(|e| BenchError::Gpu(format!("vulkan drain: {e}")))?;
            self.vk_stamp(self.vk_ts.as_ref().expect("checked").bufs[1], 1, false)?;
            // SAFETY: `self.vk_dev` is the live leaked device.
            unsafe { self.vk_dev.device_wait_idle() }
                .map_err(|e| BenchError::Gpu(format!("vulkan drain: {e}")))?;
            self.vk_read_seconds()?
        };
        let render_seconds = render_at.elapsed().as_secs_f64();
        let readback_at = Instant::now();
        let image = if readback {
            Some(read_surface(
                &mut self.prepared.as_mut().expect("checked").surface,
                Self::NAME,
                self.size.0,
                self.size.1,
            )?)
        } else {
            None
        };
        Ok(Submit {
            image,
            gpu: GpuSample::whole_frame(frame, gpu_seconds),
            phases: None,
            render_seconds: Some(render_seconds),
            readback_seconds: readback.then(|| readback_at.elapsed().as_secs_f64()),
        })
    }

    fn counters(&self) -> Counters {
        self.counters.clone()
    }

    fn device(&self) -> DeviceInfo {
        DeviceInfo {
            adapter: Some(self.device_name.clone()),
            backend: Some("vulkan".into()),
            cpu: crate::cpu_model(),
            thermal_celsius: crate::thermal_celsius(),
            ..DeviceInfo::default()
        }
    }

    fn memory(&self) -> AdapterMemory {
        AdapterMemory {
            engine: Reading::unavailable("Skia Ganesh does not expose engine memory"),
            wgpu_allocator: Reading::unavailable("Skia Vulkan does not use the wgpu allocator"),
            skia_budgeted: skia_budget("ganesh", self.dctx.resource_cache_usage().resource_bytes),
            vk_memory_budget: ash_vk_memory_budget(
                self.vk_instance,
                self.vk_physical_device,
                &self.device_name,
            ),
        }
    }
}

/// `skia-metal`: Skia Graphite on Metal, Apple platforms only.
#[cfg(all(feature = "skia-metal", target_vendor = "apple"))]
mod graphite_metal {
    // `MTLCreateSystemDefaultDevice` resolves the default device through
    // CoreGraphics; objc2-metal documents linking the framework directly
    // when `objc2-core-graphics` is not a dependency.
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {}

    use std::collections::BTreeSet;
    use std::ffi::c_void;
    use std::time::Instant;

    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2_metal::{MTLCommandBuffer, MTLCommandQueue, MTLCreateSystemDefaultDevice, MTLDevice};
    use skia_safe::gpu::graphite::{self, mtl as gmtl};
    use skia_safe::gpu::{Mipmapped, graphite::surfaces};
    use skia_safe::wrapper::PointerWrapper;
    use skia_safe::{AlphaType, ColorType, ImageInfo};

    use cherenkov_oracle::F32Image;
    use cherenkov_scene::Feature;

    use super::{SkiaPrepared, build_cmds, p3_cs, replay, skia_features, skia_missing_api};
    use crate::convert;
    use crate::memory::{AdapterMemory, Reading, skia_budget};
    use crate::{
        BenchError, Counters, DeviceInfo, EncodeInput, Engine, EngineInfo, GpuSample, Submit,
    };

    /// Skia Graphite on Metal: an offscreen `RGBAF16` linear-P3 render
    /// target on the system device, timed by command-buffer markers.
    pub struct SkiaMtl {
        info: EngineInfo,
        device: Retained<ProtocolObject<dyn MTLDevice>>,
        /// The queue Graphite submits on — the markers run on it too.
        queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
        _bctx: gmtl::BackendContext,
        ctx: graphite::Context,
        recorder: graphite::Recorder,
        /// Prepared scene state: the render target and the resolved
        /// `Cmd` list.
        prepared: Option<SkiaPrepared>,
        /// The `Recording` `encode` snapped; `submit` inserts it.
        recording: Option<graphite::Recording>,
        size: (u32, u32),
        counters: Counters,
    }

    impl SkiaMtl {
        /// Adapter key.
        pub const NAME: &'static str = "skia-metal";

        /// Creates the adapter: the system default Metal device, its
        /// command queue, and a Graphite context + recorder on them.
        ///
        /// # Errors
        /// [`BenchError::Gpu`] on any Metal or Skia failure.
        pub fn new() -> Result<Self, BenchError> {
            fn gerr(msg: &str) -> BenchError {
                BenchError::Gpu(format!("skia-metal: {msg}"))
            }
            let device = MTLCreateSystemDefaultDevice()
                .ok_or_else(|| gerr("MTLCreateSystemDefaultDevice returned nil"))?;
            let queue = device
                .newCommandQueue()
                .ok_or_else(|| gerr("newCommandQueue returned nil"))?;
            // SAFETY: `device` and `queue` are live objects retained by
            // `self`, which outlives the context.
            let bctx = unsafe {
                gmtl::BackendContext::new(
                    Retained::as_ptr(&device).cast::<c_void>().cast_mut(),
                    Retained::as_ptr(&queue).cast::<c_void>().cast_mut(),
                )
            };
            let mut ctx = gmtl::context_factory::make_metal(&bctx, None)
                .ok_or_else(|| gerr("make_metal returned None"))?;
            let recorder = ctx
                .make_recorder(None)
                .ok_or_else(|| gerr("make_recorder returned None"))?;
            Ok(Self {
                info: EngineInfo {
                    name: Self::NAME,
                    engine_crate: "skia-safe",
                    crate_version: env!("DEP_SKIA_SAFE_VERSION"),
                    source_rev: option_env!("DEP_SKIA_SAFE_SOURCE_REV").map(String::from),
                    output_format: "Graphite Metal render target, RGBAF16 premultiplied \
                                    (linear Display P3)"
                        .into(),
                    precision: "Skia blends in f32 premultiplied; output is f16 (≈11-bit mantissa)",
                    route: "metal (graphite)",
                    color_note: "scene colours converted working-space→linear-P3 f32 unclamped; \
                                 readback f16→f32",
                    encode_scope: "records the pre-resolved `Canvas` draw calls into the \
                                   Graphite `Recorder` and `snap()`s them into a `Recording` \
                                   — `submit` inserts it on the shared Metal queue",
                },
                device,
                queue,
                _bctx: bctx,
                ctx,
                recorder,
                prepared: None,
                recording: None,
                size: (0, 0),
                counters: Counters::default(),
            })
        }

        /// Commits an empty command buffer and waits for it, which on a
        /// serial `MTLCommandQueue` means every buffer committed before it
        /// — including Graphite's — has completed.
        ///
        /// # Errors
        /// [`BenchError::Gpu`] when the queue returns no command buffer.
        fn drain(&self) -> Result<(), BenchError> {
            let cb = self
                .queue
                .commandBuffer()
                .ok_or_else(|| BenchError::Gpu("metal drain: no command buffer".into()))?;
            cb.commit();
            cb.waitUntilCompleted();
            Ok(())
        }

        /// Reads the render target as premultiplied `f16` linear-P3
        /// pixels and decodes to the suite `f32` working image.
        ///
        /// # Errors
        /// [`BenchError::Gpu`] when the synchronous Graphite readback
        /// fails.
        #[expect(clippy::cast_possible_wrap, reason = "render target dims fit i32")]
        fn read_target(&mut self) -> Result<F32Image, BenchError> {
            let (w, h) = self.size;
            let info = ImageInfo::new(
                (w as i32, h as i32),
                ColorType::RGBAF16,
                AlphaType::Premul,
                p3_cs(Self::NAME)?,
            );
            let mut buf = vec![0u8; w as usize * h as usize * 8];
            if !self.ctx.read_pixels(
                &mut self
                    .prepared
                    .as_mut()
                    .expect("submit checked prepared")
                    .surface,
                &info,
                &mut buf,
                w as usize * 8,
                (0, 0),
            ) {
                return Err(BenchError::Gpu("skia-metal: read_pixels failed".into()));
            }
            let pixels = buf
                .as_chunks::<8>()
                .0
                .iter()
                .map(|q| {
                    [
                        convert::f16_to_f32(u16::from_le_bytes([q[0], q[1]])),
                        convert::f16_to_f32(u16::from_le_bytes([q[2], q[3]])),
                        convert::f16_to_f32(u16::from_le_bytes([q[4], q[5]])),
                        convert::f16_to_f32(u16::from_le_bytes([q[6], q[7]])),
                    ]
                })
                .collect();
            Ok(F32Image {
                width: w,
                height: h,
                pixels,
            })
        }
    }

    impl Engine for SkiaMtl {
        fn info(&self) -> &EngineInfo {
            &self.info
        }

        fn supported(&self) -> BTreeSet<Feature> {
            skia_features().into_iter().collect()
        }

        #[expect(clippy::cast_possible_wrap, reason = "render target dims fit i32")]
        fn prepare(&mut self, input: &EncodeInput<'_>) -> Result<(), BenchError> {
            convert::check_features(Self::NAME, input.scene, &skia_features(), skia_missing_api)?;
            self.counters = Counters::default();
            let (w, h) = (input.scene.width, input.scene.height);
            let info = ImageInfo::new(
                (w as i32, h as i32),
                ColorType::RGBAF16,
                AlphaType::Premul,
                p3_cs(Self::NAME)?,
            );
            let surface = surfaces::render_target(
                &mut self.recorder,
                &info,
                Mipmapped::No,
                None,
                Some("cherenkov-bench"),
            )
            .ok_or_else(|| BenchError::Gpu("skia-metal render target".into()))?;
            self.size = (w, h);
            self.prepared = Some(SkiaPrepared {
                surface,
                cmds: build_cmds(input.scene, input.blobs, Self::NAME, &mut self.counters)?,
            });
            Ok(())
        }

        /// Records the frame: the `Canvas` draw calls accumulate in the
        /// `Recorder` and `snap()` finalizes them into a `Recording` —
        /// Graphite's own recording API, and nothing else.
        fn encode(&mut self, _input: &EncodeInput<'_>) -> Result<(), BenchError> {
            let prepared = self
                .prepared
                .as_mut()
                .ok_or_else(|| BenchError::Engine("skia-metal: encode before prepare".into()))?;
            replay(prepared.surface.canvas(), &prepared.cmds);
            self.recording = Some(
                self.recorder
                    .snap()
                    .ok_or_else(|| BenchError::Gpu("skia-metal: snap returned None".into()))?,
            );
            Ok(())
        }

        fn submit(&mut self, frame: u64, readback: bool) -> Result<Submit, BenchError> {
            if self.prepared.is_none() {
                return Err(BenchError::Engine(
                    "skia-metal: submit before prepare".into(),
                ));
            }
            let mut recording = self
                .recording
                .take()
                .ok_or_else(|| BenchError::Engine("skia-metal: submit before encode".into()))?;
            // GPU time: bracket Graphite's insert+submit with empty marker
            // command buffers on the same serial queue, each preceded by
            // a full drain so a marker cannot overlap the render in
            // execution (command buffers on one Metal queue may run
            // concurrently on Apple GPUs and would otherwise bracket an
            // empty interval). This serializes CPU and GPU for the
            // measured frame — a synchronous probe, not a pipelined
            // frame rate.
            self.drain()?;
            let render_at = Instant::now();
            let start = self
                .queue
                .commandBuffer()
                .ok_or_else(|| BenchError::Gpu("metal marker: no command buffer".into()))?;
            start.commit();
            if !matches!(
                self.ctx
                    .insert_recording(&graphite::InsertRecordingInfo::new(&mut recording)),
                graphite::InsertStatus::Success
            ) {
                return Err(BenchError::Gpu(
                    "skia-metal: insert_recording failed".into(),
                ));
            }
            if !self.ctx.submit(None) {
                return Err(BenchError::Gpu("skia-metal: submit failed".into()));
            }
            self.drain()?;
            let end = self
                .queue
                .commandBuffer()
                .ok_or_else(|| BenchError::Gpu("metal marker: no command buffer".into()))?;
            end.commit();
            end.waitUntilCompleted();
            // The drained start marker finished before Graphite's work
            // began; the drained end marker began after it finished, so
            // [start.GPUEndTime, end.GPUStartTime] brackets it exactly.
            let (s, e) = (start.GPUEndTime(), end.GPUStartTime());
            let gpu_seconds = (s > 0.0 && e > s).then_some(e - s);
            let render_seconds = render_at.elapsed().as_secs_f64();
            let readback_at = Instant::now();
            let image = if readback {
                Some(self.read_target()?)
            } else {
                None
            };
            Ok(Submit {
                image,
                gpu: GpuSample::whole_frame(frame, gpu_seconds),
                phases: None,
                render_seconds: Some(render_seconds),
                readback_seconds: readback.then(|| readback_at.elapsed().as_secs_f64()),
            })
        }

        fn counters(&self) -> Counters {
            self.counters.clone()
        }

        fn device(&self) -> DeviceInfo {
            DeviceInfo {
                adapter: Some(self.device.name().to_string()),
                backend: Some("metal".into()),
                target_format: Some("RGBA16Float".into()),
                cpu: crate::cpu_model(),
                thermal_celsius: crate::thermal_celsius(),
                ..DeviceInfo::default()
            }
        }

        fn memory(&self) -> AdapterMemory {
            let native: &skia_bindings::skgpu_graphite_Context = PointerWrapper::inner(&self.ctx);
            let budgeted_bytes = {
                // SAFETY: the context is alive and currentBudgetedBytes is a const getter.
                unsafe { native.currentBudgetedBytes() }
            };
            AdapterMemory {
                engine: Reading::unavailable("Skia Graphite does not expose engine memory"),
                wgpu_allocator: Reading::unavailable("Skia Graphite does not use wgpu"),
                skia_budgeted: skia_budget("graphite", budgeted_bytes),
                vk_memory_budget: Reading::unavailable("Skia Graphite uses Metal, not Vulkan"),
            }
        }
    }
}

#[cfg(all(feature = "skia-metal", target_vendor = "apple"))]
pub use graphite_metal::SkiaMtl;
