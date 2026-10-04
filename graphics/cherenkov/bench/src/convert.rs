//! Shared scene → engine conversion helpers.
//!
//! Colour contract (uniform across adapters): a scene [`Color`] is first
//! converted to premultiplied linear Display P3 (the suite working space)
//! via `cherenkov_oracle::color::to_working`, then un-premultiplied,
//! converted to linear sRGB and clamped to the sRGB gamut, `srgb_encode`d
//! and quantized to `rgba8` straight alpha — the input every engine
//! accepts (`peniko::AlphaColor<Srgb>`, `Color4f` on an sRGB surface).
//! Gradient stops are instead passed through as `peniko`/`color`
//! `DynamicColor`s, preserving the declared colour space end to end where
//! the engine supports it.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use cherenkov_oracle::color::{linear_p3_to_linear_srgb, srgb_encode, to_working};
use cherenkov_scene::{
    BlendMode, Color, ColorSpace, Draw, Extend, Feature, FillRule, ImagePaint, Item, Layer,
    NormalizedCoord, Paint, ResourceHash, Scene, Shape, StrokeStyle,
};
use kurbo::{Affine, BezPath};
use peniko::color::{
    AlphaColor, ColorSpaceTag, DisplayP3, DynamicColor, LinearSrgb, Rec2020, Srgb,
};
use peniko::{
    Blob, Brush, ColorStops, Extend as PExtend, FontData, ImageBrush, ImageData, ImageSampler,
};

use crate::{BenchError, Counters};

/// Resource blobs (fonts, images) loaded for a scene, keyed by hash.
pub type Blobs = BTreeMap<ResourceHash, Vec<u8>>;

/// `F2Dot14` bit patterns per distinct normalized-coord set of one font:
/// `(declared coords, resolved bits)` pairs.
type CoordSets = Vec<(Vec<NormalizedCoord>, Vec<i16>)>;

/// Per-scene resources resolved once in [`crate::Engine::prepare`].
///
/// This is everything an application would build once and cache across
/// frames — decoded image payloads, `peniko` font data, resolved
/// variation coordinates, and GPU image handles where the adapter uploads
/// resources — so the timed per-frame `encode` covers only the engine's
/// own recording calls, not resource creation.
#[derive(Debug)]
pub struct Prepared {
    /// `peniko::FontData` per `(font hash, face index)` — an `Arc`-backed
    /// handle, not a re-parse; the blob bytes are cloned cheaply.
    fonts: HashMap<(ResourceHash, u32), FontData>,
    /// `F2Dot14` design-axis bit patterns per font hash + coord set. The
    /// skrifa `FontRef` parse that orders axes happens in `build`, not per
    /// frame.
    coords: HashMap<ResourceHash, CoordSets>,
    /// Decoded `rgba8` image payloads per image hash.
    images: HashMap<ResourceHash, ImageData>,
    /// GPU image handles per hash where the adapter uploads resources at
    /// prepare time (`vello-hybrid` atlas ids, plus a transparency hint);
    /// empty for adapters that keep image paints CPU-resident.
    gpu_images: HashMap<ResourceHash, (u32, bool)>,
}

impl Prepared {
    /// Builds the resource set for `scene`: wraps every glyph run's font
    /// in `peniko::FontData`, resolves its normalized coords through
    /// skrifa once, and decodes every referenced image.
    ///
    /// # Errors
    /// [`BenchError`] on a missing or undecodable resource.
    pub fn build(scene: &Scene, blobs: &Blobs) -> Result<Self, BenchError> {
        let mut p = Self {
            fonts: HashMap::new(),
            coords: HashMap::new(),
            images: HashMap::new(),
            gpu_images: HashMap::new(),
        };
        visit(&mut p, &scene.root, blobs)?;
        // `Paint::Image` can also appear inside glyph-run paints.
        for layer in paint_images(scene) {
            p.decode(layer, blobs)?;
        }
        Ok(p)
    }

    /// Registers a GPU-resident image handle (`vello-hybrid` atlas id).
    pub fn set_gpu_image(&mut self, hash: ResourceHash, id: u32, may_have_transparency: bool) {
        self.gpu_images.insert(hash, (id, may_have_transparency));
    }

    /// The hash + decoded payload of every prepared image (for adapters
    /// that upload resources to the GPU in `prepare`).
    pub fn image_entries(&self) -> impl Iterator<Item = (ResourceHash, &ImageData)> {
        self.images.iter().map(|(h, d)| (*h, d))
    }

    /// Decoded `peniko::ImageData` for an image hash.
    ///
    /// # Errors
    /// [`BenchError::Scene`] when the image was not prepared.
    pub fn image(&self, hash: ResourceHash) -> Result<&ImageData, BenchError> {
        self.images
            .get(&hash)
            .ok_or_else(|| cherenkov_scene::SceneError::MissingResource(hash).into())
    }

    /// Total decoded texel bytes of every prepared image (`w*h*4`) — the
    /// `bytes_uploaded` counter reports what the GPU receives, not the
    /// compressed PNG size.
    #[must_use]
    pub fn texel_bytes(&self) -> u64 {
        self.images
            .values()
            .map(|d| u64::from(d.width) * u64::from(d.height) * 4)
            .sum()
    }

    /// `peniko::FontData` for `(font hash, face index)`.
    ///
    /// # Errors
    /// [`BenchError::Scene`] when the font was not prepared.
    pub fn font(&self, hash: ResourceHash, index: u32) -> Result<FontData, BenchError> {
        self.fonts
            .get(&(hash, index))
            .cloned()
            .ok_or_else(|| cherenkov_scene::SceneError::MissingResource(hash).into())
    }

    /// Resolved `F2Dot14` design-axis bits for a run's normalized coords,
    /// in the font's axis order. Empty when the font has no axes.
    #[must_use]
    pub fn coord_bits(&self, hash: ResourceHash, coords: &[NormalizedCoord]) -> Vec<i16> {
        self.coords
            .get(&hash)
            .and_then(|v| {
                v.iter()
                    .find(|(cs, _)| cs.as_slice() == coords)
                    .map(|(_, b)| b.clone())
            })
            .unwrap_or_default()
    }

    /// Resolves a scene [`Paint`] to a `peniko::Brush`, images resolved to
    /// the decoded payload.
    ///
    /// # Errors
    /// [`BenchError`] for missing resources or unsupported extends.
    pub fn brush(&self, engine: &'static str, paint: &Paint) -> Result<Brush, BenchError> {
        self.brush_inner(engine, paint)
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "peniko gradient geometry is f32; scene geometry is f64"
    )]
    fn brush_inner(&self, engine: &'static str, paint: &Paint) -> Result<Brush, BenchError> {
        Ok(match paint {
            Paint::Transformed { .. } => {
                return Err(BenchError::Unsupported {
                    engine,
                    feature: Feature::PaintTransform,
                    api: Some("independent paint transform adapter"),
                });
            }
            Paint::Mesh(_) => {
                return Err(BenchError::Unsupported {
                    engine,
                    feature: cherenkov_scene::Feature::MeshGradient,
                    api: Some("bilinear mesh paint"),
                });
            }
            Paint::Solid(c) => Brush::Solid(peniko_solid(c)),
            Paint::Linear(g) => Brush::Gradient(gradient(
                engine,
                peniko::GradientKind::Linear(peniko::LinearGradientPosition {
                    start: g.start,
                    end: g.end,
                }),
                &g.stops,
                g.extend,
                g.interpolation,
            )?),
            Paint::Radial(g) => Brush::Gradient(gradient(
                engine,
                peniko::GradientKind::Radial(peniko::RadialGradientPosition {
                    start_center: g.center0,
                    start_radius: g.r0 as f32,
                    end_center: g.center1,
                    end_radius: g.r1 as f32,
                }),
                &g.stops,
                g.extend,
                g.interpolation,
            )?),
            Paint::Sweep(g) => Brush::Gradient(gradient(
                engine,
                peniko::GradientKind::Sweep(peniko::SweepGradientPosition {
                    center: g.center,
                    start_angle: g.start_angle as f32,
                    end_angle: g.end_angle as f32,
                }),
                &g.stops,
                g.extend,
                g.interpolation,
            )?),
            Paint::Image(ip) => Brush::Image(ImageBrush {
                image: self.image(ip.image)?.clone(),
                sampler: ImageSampler {
                    x_extend: extend(engine, ip.extend_x)?,
                    y_extend: extend(engine, ip.extend_y)?,
                    quality: match ip.sampling {
                        cherenkov_scene::Sampling::Nearest => peniko::ImageQuality::Low,
                        cherenkov_scene::Sampling::Bilinear => peniko::ImageQuality::Medium,
                    },
                    alpha: 1.0,
                },
            }),
        })
    }

    /// An `ImageSource` for `vello_cpu`/`vello_hybrid` paints: the atlas
    /// `OpaqueId` when the adapter uploaded the image in `prepare`, else
    /// the decoded pixmap (the CPU-resident route).
    ///
    /// # Errors
    /// [`BenchError::Scene`] when the image was not prepared.
    #[cfg(any(feature = "vello-cpu", feature = "vello-hybrid"))]
    pub fn image_source(
        &self,
        hash: ResourceHash,
    ) -> Result<vello_common::paint::ImageSource, BenchError> {
        if let Some(&(id, transp)) = self.gpu_images.get(&hash) {
            return Ok(
                vello_common::paint::ImageSource::opaque_id_with_transparency_hint(
                    vello_common::paint::ImageId::new(id),
                    transp,
                ),
            );
        }
        Ok(vello_common::paint::ImageSource::from_peniko_image_data(
            self.image(hash)?,
        ))
    }

    /// `vello_common::paint::PaintType` — the paint type
    /// `vello_cpu`/`vello_hybrid` `set_paint` takes, with images resolved
    /// through [`Prepared::image_source`].
    ///
    /// # Errors
    /// [`BenchError`] for missing resources or unsupported extends.
    #[cfg(any(feature = "vello-cpu", feature = "vello-hybrid"))]
    pub fn paint_type(
        &self,
        engine: &'static str,
        paint: &Paint,
    ) -> Result<vello_common::paint::PaintType, BenchError> {
        use vello_common::paint::PaintType;
        Ok(match paint {
            Paint::Image(ip) => PaintType::Image(peniko::ImageBrush {
                image: self.image_source(ip.image)?,
                sampler: ImageSampler {
                    x_extend: extend(engine, ip.extend_x)?,
                    y_extend: extend(engine, ip.extend_y)?,
                    quality: match ip.sampling {
                        cherenkov_scene::Sampling::Nearest => peniko::ImageQuality::Low,
                        cherenkov_scene::Sampling::Bilinear => peniko::ImageQuality::Medium,
                    },
                    alpha: 1.0,
                },
            }),
            _ => to_paint_type(self.brush_inner(engine, paint)?),
        })
    }

    fn decode(&mut self, hash: ResourceHash, blobs: &Blobs) -> Result<(), BenchError> {
        if self.images.contains_key(&hash) {
            return Ok(());
        }
        let bytes = blobs
            .get(&hash)
            .ok_or(cherenkov_scene::SceneError::MissingResource(hash))?;
        self.images.insert(hash, image_data(bytes)?);
        Ok(())
    }
}

/// Walks `layer` registering every font (plus coord sets) and every
/// top-level `Paint::Image`/`Draw::Image` payload into `p`.
fn visit(p: &mut Prepared, layer: &Layer, blobs: &Blobs) -> Result<(), BenchError> {
    for item in &layer.items {
        match item {
            Item::Layer(l) => visit(p, l, blobs)?,
            Item::Group(g) => visit_group(p, g, blobs)?,
            Item::Draw(d) => visit_draw(p, d, blobs)?,
        }
    }
    Ok(())
}

/// [`visit`] over a group's member list (draws and nested groups only).
fn visit_group(
    p: &mut Prepared,
    group: &cherenkov_scene::Group,
    blobs: &Blobs,
) -> Result<(), BenchError> {
    for item in &group.items {
        match item {
            cherenkov_scene::GroupItem::Draw(d) => visit_draw(p, d, blobs)?,
            cherenkov_scene::GroupItem::Group(g) => visit_group(p, g, blobs)?,
        }
    }
    Ok(())
}

/// One draw's fonts and image payloads, registered into `p`.
fn visit_draw(p: &mut Prepared, d: &Draw, blobs: &Blobs) -> Result<(), BenchError> {
    match d {
        Draw::Glyphs(run) => {
            let bytes = blobs
                .get(&run.font)
                .ok_or(cherenkov_scene::SceneError::MissingResource(run.font))?;
            p.fonts
                .entry((run.font, run.font_index))
                .or_insert_with(|| {
                    FontData::new(Blob::new(Arc::new(bytes.clone())), run.font_index)
                });
            let coords = coord_bits(bytes, &run.normalized_coords);
            let entry = p.coords.entry(run.font).or_default();
            if entry.iter().all(|(cs, _)| cs != &run.normalized_coords) {
                entry.push((run.normalized_coords.clone(), coords));
            }
        }
        Draw::Fill { paint, .. } | Draw::Stroke { paint, .. } => {
            if let Some(ip) = image_paint(paint) {
                p.decode(ip.image, blobs)?;
            }
        }
        Draw::Shadow { .. } => {}
        Draw::Image { image, .. } => {
            p.decode(*image, blobs)?;
        }
    }
    Ok(())
}

/// The `Paint::Image` inside a draw's paint, when any.
#[must_use]
pub fn image_paint(paint: &Paint) -> Option<&ImagePaint> {
    match paint {
        Paint::Transformed { paint, .. } => image_paint(paint),
        Paint::Image(ip) => Some(ip),
        _ => None,
    }
}

/// Image hashes referenced by `Paint::Image` inside glyph-run paints
/// (the draw walk above only inspects top-level paints).
fn paint_images(scene: &Scene) -> Vec<ResourceHash> {
    fn visit_draw(d: &Draw, out: &mut Vec<ResourceHash>) {
        if let Draw::Glyphs(run) = d
            && let Some(ip) = image_paint(&run.paint)
        {
            out.push(ip.image);
        }
    }
    fn visit_group(group: &cherenkov_scene::Group, out: &mut Vec<ResourceHash>) {
        for item in &group.items {
            match item {
                cherenkov_scene::GroupItem::Draw(d) => visit_draw(d, out),
                cherenkov_scene::GroupItem::Group(g) => visit_group(g, out),
            }
        }
    }
    fn visit(layer: &Layer, out: &mut Vec<ResourceHash>) {
        for item in &layer.items {
            match item {
                Item::Layer(l) => visit(l, out),
                Item::Draw(d) => visit_draw(d, out),
                Item::Group(g) => visit_group(g, out),
            }
        }
    }
    let mut out = Vec::new();
    visit(&scene.root, &mut out);
    out
}

/// Loads every resource `scene` references from `dir`.
///
/// # Errors
/// [`BenchError::Scene`] on missing blobs or I/O failure.
pub fn load_blobs(scene: &Scene, dir: &std::path::Path) -> Result<Blobs, BenchError> {
    let mut out = Blobs::new();
    for hash in scene.resource_refs() {
        out.insert(hash, Scene::resource(dir, hash)?);
    }
    Ok(out)
}

/// Errors when the scene declares a feature the adapter does not implement.
/// `api_for` names the upstream API the engine lacks for a missing feature.
///
/// # Errors
/// [`BenchError::Unsupported`] naming the first missing feature.
pub fn check_features(
    engine: &'static str,
    scene: &Scene,
    supported: &[Feature],
    api_for: impl Fn(&Feature) -> Option<&'static str>,
) -> Result<(), BenchError> {
    for f in &scene.features {
        if !supported.contains(f) {
            return Err(BenchError::Unsupported {
                engine,
                feature: f.clone(),
                api: api_for(f),
            });
        }
    }
    Ok(())
}

/// Converts a scene colour to `rgba8` straight alpha (sRGB encoded).
///
/// Wide-gamut channels are clamped to the sRGB gamut; HDR channels are
/// clamped to `0.0..=1.0`.
#[must_use]
#[allow(clippy::many_single_char_names)] // r/g/b/a channel names
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "channels are clamped to [0,1] before the deliberate u8 quantize"
)]
pub fn straight_srgb8(c: &Color) -> [u8; 4] {
    let [r, g, b, a] = to_working(c);
    let [lr, lg, lb] = if a > 1e-12 {
        [r / a, g / a, b / a]
    } else {
        [0.0; 3]
    };
    let [sr, sg, sb] = linear_p3_to_linear_srgb([lr, lg, lb]);
    [
        (srgb_encode(sr.clamp(0.0, 1.0)) * 255.0).round() as u8,
        (srgb_encode(sg.clamp(0.0, 1.0)) * 255.0).round() as u8,
        (srgb_encode(sb.clamp(0.0, 1.0)) * 255.0).round() as u8,
        (a.clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

/// Converts a scene colour to a `peniko` `AlphaColor<Srgb>`.
#[must_use]
#[allow(clippy::many_single_char_names)] // r/g/b/a channel names
pub fn peniko_solid(c: &Color) -> AlphaColor<Srgb> {
    let [r, g, b, a] = straight_srgb8(c);
    AlphaColor::from_rgba8(r, g, b, a)
}

/// Preserves a scene colour's declared space in a `DynamicColor` for
/// gradient stops.
#[must_use]
#[allow(clippy::many_single_char_names)] // r/g/b/a channel names
#[expect(
    clippy::cast_possible_truncation,
    reason = "peniko colours are f32; the scene's f64 channel math narrows"
)]
pub fn dynamic_color(c: &Color) -> DynamicColor {
    let [r, g, b, a] = c.components;
    match c.space {
        ColorSpace::Srgb => DynamicColor::from_alpha_color(AlphaColor::<Srgb>::new([r, g, b, a])),
        ColorSpace::DisplayP3 => {
            DynamicColor::from_alpha_color(AlphaColor::<DisplayP3>::new([r, g, b, a]))
        }
        ColorSpace::LinearSrgb => {
            DynamicColor::from_alpha_color(AlphaColor::<LinearSrgb>::new([r, g, b, a]))
        }
        // `peniko`/`color` 0.3 has no LinearP3 tag: express the colour in
        // linear sRGB instead — the same linear-light space vello evaluates
        // gradients in — so the value survives without a gamut shift beyond
        // the P3→sRGB matrix the engines apply anyway.
        ColorSpace::LinearP3 => {
            let [sr, sg, sb] = linear_p3_to_linear_srgb([f64::from(r), f64::from(g), f64::from(b)]);
            DynamicColor::from_alpha_color(AlphaColor::<LinearSrgb>::new([
                sr as f32, sg as f32, sb as f32, a,
            ]))
        }
        ColorSpace::Rec2020 => {
            DynamicColor::from_alpha_color(AlphaColor::<Rec2020>::new([r, g, b, a]))
        }
    }
}

/// The upstream API vello-family adapters lack for [`Extend::None`]:
/// `peniko::Extend` offers only `Pad`/`Repeat`/`Reflect` (peniko 0.6).
///
/// Skia does have it (`SkTileMode::kDecal`), so this is a `peniko` gap, not
/// an inherent engine limitation.
pub const EXTEND_NONE_API: &str =
    "peniko::Extend has no None variant (peniko offers Pad/Repeat/Reflect only)";

/// Maps a scene [`Extend`] to `peniko`'s `Extend` (`Pad`, `Repeat`,
/// `Reflect`). `None` (transparent outside the domain) has no `peniko`
/// equivalent and is reported as unsupported by the adapter.
///
/// # Errors
/// [`BenchError::Unsupported`] for [`Extend::None`].
pub const fn extend(engine: &'static str, e: Extend) -> Result<PExtend, BenchError> {
    match e {
        Extend::Pad => Ok(PExtend::Pad),
        Extend::Repeat => Ok(PExtend::Repeat),
        Extend::Reflect => Ok(PExtend::Reflect),
        Extend::None => Err(BenchError::Unsupported {
            engine,
            feature: Feature::ExtendNone,
            api: Some(EXTEND_NONE_API),
        }),
    }
}

/// `peniko` adapters cannot express `linear-p3` gradient interpolation.
///
/// The upstream API missing for [`Feature::InterpolationSpace`]
/// (LinearP3): `color` 0.3's `ColorSpaceTag` has no linear-P3 variant, so
/// a gradient declaring `linear-p3` interpolation cannot be expressed and
/// must be reported unsupported rather than silently remapped to linear
/// sRGB.
pub const LINEAR_P3_API: &str = "peniko::color::ColorSpaceTag has no linear-P3 variant";

/// Maps the scene's interpolation colour space onto a `ColorSpaceTag`,
/// `None` for [`ColorSpace::LinearP3`] (no tag — see [`LINEAR_P3_API`]).
#[must_use]
pub const fn interpolation_tag(space: ColorSpace) -> Option<ColorSpaceTag> {
    Some(match space {
        ColorSpace::Srgb => ColorSpaceTag::Srgb,
        ColorSpace::DisplayP3 => ColorSpaceTag::DisplayP3,
        ColorSpace::LinearSrgb => ColorSpaceTag::LinearSrgb,
        ColorSpace::LinearP3 => return None,
        ColorSpace::Rec2020 => ColorSpaceTag::Rec2020,
    })
}

/// Builds `peniko` colour stops preserving declared colour spaces.
#[must_use]
pub fn color_stops(stops: &[cherenkov_scene::GradientStop]) -> ColorStops {
    let v: Vec<peniko::ColorStop> = stops
        .iter()
        .map(|s| (s.offset, dynamic_color(&s.color)).into())
        .collect();
    ColorStops::from(&v[..])
}

/// Builds a `peniko` gradient from the scene description.
///
/// # Errors
/// [`BenchError::Unsupported`] for [`Extend::None`].
pub fn gradient(
    engine: &'static str,
    kind: peniko::GradientKind,
    stops: &[cherenkov_scene::GradientStop],
    ext: Extend,
    interpolation: ColorSpace,
) -> Result<peniko::Gradient, BenchError> {
    let interpolation_cs = interpolation_tag(interpolation).ok_or(BenchError::Unsupported {
        engine,
        feature: Feature::InterpolationSpace(interpolation),
        api: Some(LINEAR_P3_API),
    })?;
    Ok(peniko::Gradient {
        kind,
        extend: extend(engine, ext)?,
        interpolation_cs,
        stops: color_stops(stops),
        ..Default::default()
    })
}

/// Decodes a PNG blob into straight `rgba8` pixels — every colour type,
/// via the shared [`cherenkov_oracle::image::decode_png_rgba8`].
///
/// # Errors
/// [`BenchError::Engine`] on decode failure.
pub fn decode_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), BenchError> {
    cherenkov_oracle::image::decode_png_rgba8(bytes)
        .map_err(|e| BenchError::Engine(format!("png decode: {e}")))
}

/// Decoded texel byte count (`w*h*4`) of an image blob — the `bytes_uploaded`
/// counter reports what the GPU receives, not the compressed PNG size.
///
/// # Errors
/// [`BenchError::Engine`] on decode failure.
pub fn decoded_texel_bytes(bytes: &[u8]) -> Result<u64, BenchError> {
    let (w, h, _) = decode_png(bytes)?;
    Ok(u64::from(w) * u64::from(h) * 4)
}

/// Builds `peniko::ImageData` (straight alpha RGBA8) from a PNG blob.
///
/// # Errors
/// [`BenchError::Engine`] on decode failure.
pub fn image_data(png_bytes: &[u8]) -> Result<ImageData, BenchError> {
    let (w, h, rgba) = decode_png(png_bytes)?;
    Ok(ImageData {
        data: Blob::new(Arc::new(rgba)),
        format: peniko::ImageFormat::Rgba8,
        alpha_type: peniko::ImageAlphaType::Alpha,
        width: w,
        height: h,
    })
}

/// Resolves an image paint to a `peniko::ImageBrush`.
///
/// # Errors
/// [`BenchError::Engine`] when the blob is missing or undecodable, or
/// [`BenchError::Unsupported`] for [`Extend::None`].
pub fn image_brush(
    engine: &'static str,
    ip: &ImagePaint,
    blobs: &Blobs,
) -> Result<ImageBrush, BenchError> {
    let bytes = blobs.get(&ip.image).ok_or(BenchError::Scene(
        cherenkov_scene::SceneError::MissingResource(ip.image),
    ))?;
    Ok(ImageBrush {
        image: image_data(bytes)?,
        sampler: ImageSampler {
            x_extend: extend(engine, ip.extend_x)?,
            y_extend: extend(engine, ip.extend_y)?,
            quality: match ip.sampling {
                cherenkov_scene::Sampling::Nearest => peniko::ImageQuality::Low,
                cherenkov_scene::Sampling::Bilinear => peniko::ImageQuality::Medium,
            },
            alpha: 1.0,
        },
    })
}

/// Resolves a scene [`Paint`] to a `peniko::Brush`.
///
/// # Errors
/// [`BenchError`] for missing resources or unsupported extends.
#[expect(
    clippy::cast_possible_truncation,
    reason = "peniko gradient geometry is f32; scene geometry is f64"
)]
pub fn brush(engine: &'static str, paint: &Paint, blobs: &Blobs) -> Result<Brush, BenchError> {
    Ok(match paint {
        Paint::Transformed { .. } => {
            return Err(BenchError::Unsupported {
                engine,
                feature: Feature::PaintTransform,
                api: Some("independent paint transform adapter"),
            });
        }
        Paint::Mesh(_) => {
            return Err(BenchError::Unsupported {
                engine,
                feature: cherenkov_scene::Feature::MeshGradient,
                api: Some("bilinear mesh paint"),
            });
        }
        Paint::Solid(c) => Brush::Solid(peniko_solid(c)),
        Paint::Linear(g) => Brush::Gradient(gradient(
            engine,
            peniko::GradientKind::Linear(peniko::LinearGradientPosition {
                start: g.start,
                end: g.end,
            }),
            &g.stops,
            g.extend,
            g.interpolation,
        )?),
        Paint::Radial(g) => Brush::Gradient(gradient(
            engine,
            peniko::GradientKind::Radial(peniko::RadialGradientPosition {
                start_center: g.center0,
                start_radius: g.r0 as f32,
                end_center: g.center1,
                end_radius: g.r1 as f32,
            }),
            &g.stops,
            g.extend,
            g.interpolation,
        )?),
        Paint::Sweep(g) => Brush::Gradient(gradient(
            engine,
            peniko::GradientKind::Sweep(peniko::SweepGradientPosition {
                center: g.center,
                start_angle: g.start_angle as f32,
                end_angle: g.end_angle as f32,
            }),
            &g.stops,
            g.extend,
            g.interpolation,
        )?),
        Paint::Image(ip) => Brush::Image(image_brush(engine, ip, blobs)?),
    })
}

/// Builds `vello_common::paint::PaintType` — the paint type
/// `vello_cpu`/`vello_hybrid` `set_paint` actually takes (an image brush over
/// `vello_common::paint::ImageSource`, not plain `peniko::ImageData`).
///
/// # Errors
/// [`BenchError`] for missing resources or unsupported extends.
#[cfg(any(feature = "vello-cpu", feature = "vello-hybrid"))]
pub fn paint_type(
    engine: &'static str,
    paint: &Paint,
    blobs: &Blobs,
) -> Result<vello_common::paint::PaintType, BenchError> {
    Ok(to_paint_type(brush(engine, paint, blobs)?))
}

/// Converts a default-generic `peniko::Brush` into
/// `vello_common::paint::PaintType`, converting the image payload.
///
/// # Panics
/// [`vello_common::paint::ImageSource::from_peniko_image_data`] panics for
/// images larger than `u16::MAX` in either dimension.
#[cfg(any(feature = "vello-cpu", feature = "vello-hybrid"))]
#[must_use]
pub fn to_paint_type(brush: Brush) -> vello_common::paint::PaintType {
    match brush {
        Brush::Solid(c) => vello_common::paint::PaintType::Solid(c),
        Brush::Gradient(g) => vello_common::paint::PaintType::Gradient(g),
        Brush::Image(ib) => vello_common::paint::PaintType::Image(peniko::ImageBrush {
            image: vello_common::paint::ImageSource::from_peniko_image_data(&ib.image),
            sampler: ib.sampler,
        }),
    }
}

/// The brush-space transform a paint needs (`ImagePaint::transform` only).
#[must_use]
pub const fn brush_transform(paint: &Paint) -> Option<Affine> {
    match paint {
        Paint::Image(ip) => Some(ip.transform),
        _ => None,
    }
}

/// The transform that scales an `iw`×`ih` image to fill `dst`.
#[must_use]
pub fn image_draw_transform(iw: u32, ih: u32, dst: kurbo::Rect) -> Affine {
    Affine::translate((dst.x0, dst.y0))
        * Affine::scale_non_uniform(dst.width() / f64::from(iw), dst.height() / f64::from(ih))
}

/// Resolves a font blob + index into `peniko::FontData`.
///
/// # Errors
/// [`BenchError::Scene`] when the blob is missing.
pub fn font_data(
    blobs: &Blobs,
    hash: ResourceHash,
    index: u32,
) -> Result<peniko::FontData, BenchError> {
    let bytes = blobs
        .get(&hash)
        .ok_or(cherenkov_scene::SceneError::MissingResource(hash))?;
    Ok(peniko::FontData::new(
        Blob::new(Arc::new(bytes.clone())),
        index,
    ))
}

/// Scene normalized coords → `F2Dot14` bit patterns ordered by the font's
/// variation axes (the order all engines expect).
///
/// Unknown axes fall back to the axis default; axes absent from `coords`
/// fall back to `0` (font default).
#[must_use]
pub fn coord_bits(font_bytes: &[u8], coords: &[NormalizedCoord]) -> Vec<i16> {
    use skrifa::MetadataProvider;
    let Ok(font) = skrifa::FontRef::new(font_bytes) else {
        return Vec::new();
    };
    font.axes()
        .iter()
        .map(|axis| {
            let tag = axis.tag().to_string();
            // An axis absent from `coords` is normalized `0` (the font
            // default) — the axis's user-space default such as `400`
            // would saturate the `F2Dot14` range around `±2`.
            let v = coords
                .iter()
                .find(|c| c.tag == tag)
                .map_or(0.0, |c| c.value);
            skrifa::raw::types::F2Dot14::from_f32(v).to_bits()
        })
        .collect()
}

/// Rasterizes a shape to a `kurbo` path (`Rect`, `RoundedRect`,
/// `Continuous`, `Circle`, `Ellipse`, `Line`, `Path`).
#[must_use]
pub fn shape_path(shape: &Shape) -> BezPath {
    shape.to_path()
}

/// `peniko::ImageSampler` for `Draw::Image` — `Pad` extends and the
/// declared sampling quality. (`Paint::Image` samplers take their
/// extends from the paint instead.)
#[must_use]
pub const fn image_sampler(sampling: cherenkov_scene::Sampling) -> ImageSampler {
    ImageSampler {
        x_extend: peniko::Extend::Pad,
        y_extend: peniko::Extend::Pad,
        quality: match sampling {
            cherenkov_scene::Sampling::Nearest => peniko::ImageQuality::Low,
            cherenkov_scene::Sampling::Bilinear => peniko::ImageQuality::Medium,
        },
        alpha: 1.0,
    }
}

/// Scene stroke style → `kurbo::Stroke`.
#[must_use]
pub fn stroke(style: &StrokeStyle) -> kurbo::Stroke {
    kurbo::Stroke::from(style)
}

/// Scene blend mode → `peniko::BlendMode`: a `Mix` over `SrcOver` for the
/// W3C blend modes, `Mix::Normal` over the matching `Compose` for the
/// Porter-Duff compositing operators and plus-lighter.
#[must_use]
pub const fn blend(m: BlendMode) -> peniko::BlendMode {
    use peniko::{Compose, Mix};
    let compose = match m {
        BlendMode::Clear => Compose::Clear,
        BlendMode::Src => Compose::Copy,
        BlendMode::Dst => Compose::Dest,
        BlendMode::DestOver => Compose::DestOver,
        BlendMode::SrcIn => Compose::SrcIn,
        BlendMode::DestIn => Compose::DestIn,
        BlendMode::SrcOut => Compose::SrcOut,
        BlendMode::DestOut => Compose::DestOut,
        BlendMode::SrcAtop => Compose::SrcAtop,
        BlendMode::DestAtop => Compose::DestAtop,
        BlendMode::Xor => Compose::Xor,
        BlendMode::PlusLighter => Compose::PlusLighter,
        _ => {
            let mix = match m {
                BlendMode::Normal => Mix::Normal,
                BlendMode::Multiply => Mix::Multiply,
                BlendMode::Screen => Mix::Screen,
                BlendMode::Overlay => Mix::Overlay,
                BlendMode::Darken => Mix::Darken,
                BlendMode::Lighten => Mix::Lighten,
                BlendMode::ColorDodge => Mix::ColorDodge,
                BlendMode::ColorBurn => Mix::ColorBurn,
                BlendMode::HardLight => Mix::HardLight,
                BlendMode::SoftLight => Mix::SoftLight,
                BlendMode::Difference => Mix::Difference,
                BlendMode::Exclusion => Mix::Exclusion,
                BlendMode::Hue => Mix::Hue,
                BlendMode::Saturation => Mix::Saturation,
                BlendMode::Color => Mix::Color,
                BlendMode::Luminosity => Mix::Luminosity,
                _ => unreachable!(),
            };
            return peniko::BlendMode::new(mix, Compose::SrcOver);
        }
    };
    peniko::BlendMode::new(Mix::Normal, compose)
}

/// Scene fill rule → `peniko::Fill`.
#[must_use]
pub const fn fill(r: FillRule) -> peniko::Fill {
    match r {
        FillRule::NonZero => peniko::Fill::NonZero,
        FillRule::EvenOdd => peniko::Fill::EvenOdd,
    }
}

/// Converts an `rgba8` premultiplied sRGB buffer (engine readback) into
/// premultiplied linear-P3 `f32` pixels — the suite interchange.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "the f32 interchange image deliberately narrows the f64 pipeline"
)]
pub fn rgba8_to_working(width: u32, height: u32, rgba8: &[u8]) -> cherenkov_oracle::F32Image {
    use cherenkov_oracle::color::linear_srgb_to_linear_p3;
    let mut img = cherenkov_oracle::F32Image::new(width, height);
    for (px, out) in rgba8.as_chunks::<4>().0.iter().zip(img.pixels.iter_mut()) {
        let a = f64::from(px[3]) / 255.0;
        let lin = [
            srgb_decode(f64::from(px[0]) / 255.0),
            srgb_decode(f64::from(px[1]) / 255.0),
            srgb_decode(f64::from(px[2]) / 255.0),
        ];
        let p3 = linear_srgb_to_linear_p3(lin);
        *out = [p3[0] as f32, p3[1] as f32, p3[2] as f32, a as f32];
    }
    img
}

/// sRGB transfer-function decode (inverse of `srgb_encode`).
#[must_use]
fn srgb_decode(e: f64) -> f64 {
    if e <= 0.04045 {
        e / 12.92
    } else {
        ((e + 0.055) / 1.055).powf(2.4)
    }
}

/// IEEE 754 binary16 → `f32`, used by the skia-vulkan/`RGBAF16` readback.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "f16→f64→f32 round trip is the conversion's contract"
)]
pub fn f16_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits >> 15) & 1;
    let exp = u32::from(bits >> 10) & 0x1f;
    let frac = f64::from(bits & 0x3ff);
    let v = match exp {
        0 => frac * 2f64.powi(-24),
        0x1f => {
            if frac == 0.0 {
                f64::INFINITY
            } else {
                f64::NAN
            }
        }
        e => (1.0 + frac / 1024.0) * 2f64.powi(e.cast_signed() - 15),
    };
    (if sign == 1 { -v } else { v }) as f32
}

/// Counts draw commands and nested layers inside a layer tree (adapter
/// counters — every adapter reports what it actually issued).
pub fn count_layer(layer: &Layer, counters: &mut Counters) {
    counters.layers += 1;
    for item in &layer.items {
        match item {
            Item::Layer(l) => count_layer(l, counters),
            Item::Draw(
                Draw::Fill { .. }
                | Draw::Stroke { .. }
                | Draw::Shadow { .. }
                | Draw::Glyphs(_)
                | Draw::Image { .. },
            ) => counters.draw_commands += 1,
            Item::Group(g) => count_group(g, counters),
        }
    }
}

/// [`count_layer`] over a group's member list.
fn count_group(group: &cherenkov_scene::Group, counters: &mut Counters) {
    for item in &group.items {
        match item {
            cherenkov_scene::GroupItem::Draw(_) => counters.draw_commands += 1,
            cherenkov_scene::GroupItem::Group(g) => count_group(g, counters),
        }
    }
}

// ---------------------------------------------------------------------
// Scene → front-end recording ops, shared by the `cherenkov` (GPU) and
// `cherenkov-cpu` adapters. The adapters differ only in [`Front`]: the
// `BenchError` owner name, the fill-rule lowering (`core_rule` vs
// `front_rule`), and the interpolation `api` text.

#[cfg(any(feature = "cherenkov", feature = "cherenkov-cpu"))]
mod front {
    use std::collections::HashMap;

    use cherenkov_oracle::color::to_working;
    use cherenkov_scene::{
        BlendMode, Color, ColorSpace, Draw, Extend, Feature, FillRule, Paint, ResourceHash, Shape,
    };
    use kurbo::{BezPath, Circle, Ellipse, Line, Rect, RoundedRect};

    use super::{Blobs, coord_bits};
    use crate::BenchError;

    // ---------------------------------------------------------------------
    // Scene → front-end recording ops, shared by the `cherenkov` (GPU) and
    // `cherenkov-cpu` adapters. The adapters differ only in [`Front`]: the
    // `BenchError` owner name, the fill-rule lowering (`core_rule` vs
    // `front_rule`), and the interpolation `api` text.

    /// The adapter-specific constants the shared lowering needs.
    pub struct Front {
        /// `BenchError::Unsupported`'s `engine` (`Cherenkov::NAME`).
        pub engine: &'static str,
        /// Scene fill rule → the engine's.
        pub fill_rule: fn(FillRule) -> cherenkov::FillRule,
        /// The `api` text reported for `Feature::InterpolationSpace` — the
        /// adapters' `missing_api` strings differ.
        pub interpolation_api: Option<&'static str>,
    }

    /// The upstream API a front end lacks for a declared scene feature.
    #[must_use]
    pub const fn missing_api(front: &Front, f: &Feature) -> Option<&'static str> {
        match f {
            Feature::InterpolationSpace(_) => front.interpolation_api,
            _ => None,
        }
    }

    /// A scene shape in a form the front-end accepts.
    pub enum ShapeKind {
        Rect(Rect),
        RoundedRect(RoundedRect),
        Continuous(cherenkov::ContinuousRect),
        Circle(Circle),
        Ellipse(Ellipse),
        Line(Line),
        /// A general path; the fill rule rides on the op, not the shape.
        Path {
            path: BezPath,
            data: cherenkov::ShapeData,
        },
    }

    /// One recording step of a content layer, resolved in `prepare`.
    pub enum Op {
        /// `Draw::Fill`.
        Fill {
            /// The shape.
            shape: ShapeKind,
            /// The fill rule.
            rule: FillRule,
            /// The paint.
            paint: cherenkov::Paint,
        },
        /// `Draw::Stroke`.
        Stroke {
            /// The shape.
            shape: ShapeKind,
            /// The stroke style.
            stroke: kurbo::Stroke,
            /// The paint.
            paint: cherenkov::Paint,
        },
        /// `Draw::Shadow`.
        Shadow {
            /// The shape.
            shape: ShapeKind,
            /// The shadow.
            shadow: cherenkov::Shadow,
        },
        /// `Draw::Glyphs`.
        Glyphs {
            /// The run.
            run: cherenkov::GlyphRun,
            /// The paint.
            paint: cherenkov::Paint,
        },
        /// `Draw::Image`.
        Image {
            /// The registered image.
            image: cherenkov::ImageId,
            /// Destination rect.
            dst: Rect,
            /// Sampling.
            sampling: cherenkov::Sampling,
        },
        /// `Item::Group` — a `c.group` scope.
        Group {
            /// The group properties.
            group: cherenkov::Group,
            /// The member ops.
            ops: Vec<Self>,
        },
        /// A text layer's source — a `draw_text` draw of the parley layout.
        Text {
            /// The layout, its fonts registered.
            layout: cherenkov::TextLayout,
            /// Where the layout's top-left corner lands.
            origin: kurbo::Point,
        },
    }

    /// The front-end blend mode matching a scene mode one-for-one by name.
    #[must_use]
    pub const fn engine_blend(m: BlendMode) -> cherenkov::BlendMode {
        match m {
            BlendMode::Normal => cherenkov::BlendMode::Normal,
            BlendMode::Multiply => cherenkov::BlendMode::Multiply,
            BlendMode::Screen => cherenkov::BlendMode::Screen,
            BlendMode::Overlay => cherenkov::BlendMode::Overlay,
            BlendMode::Darken => cherenkov::BlendMode::Darken,
            BlendMode::Lighten => cherenkov::BlendMode::Lighten,
            BlendMode::ColorDodge => cherenkov::BlendMode::ColorDodge,
            BlendMode::ColorBurn => cherenkov::BlendMode::ColorBurn,
            BlendMode::HardLight => cherenkov::BlendMode::HardLight,
            BlendMode::SoftLight => cherenkov::BlendMode::SoftLight,
            BlendMode::Difference => cherenkov::BlendMode::Difference,
            BlendMode::Exclusion => cherenkov::BlendMode::Exclusion,
            BlendMode::Hue => cherenkov::BlendMode::Hue,
            BlendMode::Saturation => cherenkov::BlendMode::Saturation,
            BlendMode::Color => cherenkov::BlendMode::Color,
            BlendMode::Luminosity => cherenkov::BlendMode::Luminosity,
            BlendMode::Clear => cherenkov::BlendMode::Clear,
            BlendMode::Src => cherenkov::BlendMode::Src,
            BlendMode::Dst => cherenkov::BlendMode::Dst,
            BlendMode::DestOver => cherenkov::BlendMode::DestOver,
            BlendMode::SrcIn => cherenkov::BlendMode::SrcIn,
            BlendMode::DestIn => cherenkov::BlendMode::DestIn,
            BlendMode::SrcOut => cherenkov::BlendMode::SrcOut,
            BlendMode::DestOut => cherenkov::BlendMode::DestOut,
            BlendMode::SrcAtop => cherenkov::BlendMode::SrcAtop,
            BlendMode::DestAtop => cherenkov::BlendMode::DestAtop,
            BlendMode::Xor => cherenkov::BlendMode::Xor,
            BlendMode::PlusLighter => cherenkov::BlendMode::PlusLighter,
        }
    }

    /// A scene colour → the front-end's straight-alpha working colour.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::many_single_char_names,
        reason = "the working space is f32 at the engine boundary"
    )]
    #[must_use]
    pub fn working(c: &Color) -> cherenkov::WorkingColor {
        let [r, g, b, a] = to_working(c);
        let (r, g, b) = if a > 1e-12 {
            (r / a, g / a, b / a)
        } else {
            (0.0, 0.0, 0.0)
        };
        cherenkov::WorkingColor::new([r as f32, g as f32, b as f32, a as f32])
    }

    const fn front_extend(e: Extend) -> cherenkov::Extend {
        match e {
            Extend::Pad => cherenkov::Extend::Pad,
            Extend::Repeat => cherenkov::Extend::Repeat,
            Extend::Reflect => cherenkov::Extend::Reflect,
            Extend::None => cherenkov::Extend::None,
        }
    }

    const fn interpolation(
        front: &Front,
        space: ColorSpace,
    ) -> Result<cherenkov::Interpolation, BenchError> {
        match space {
            ColorSpace::Srgb => Ok(cherenkov::Interpolation::SrgbEncoded),
            ColorSpace::LinearP3 | ColorSpace::LinearSrgb => Ok(cherenkov::Interpolation::Working),
            space => Err(BenchError::Unsupported {
                engine: front.engine,
                feature: Feature::InterpolationSpace(space),
                api: front.interpolation_api,
            }),
        }
    }

    fn stops(stops: &[cherenkov_scene::GradientStop]) -> Vec<cherenkov::ColorStop> {
        stops
            .iter()
            .map(|s| cherenkov::ColorStop {
                offset: s.offset,
                color: working(&s.color),
            })
            .collect()
    }

    /// A scene paint → the front-end paint.
    ///
    /// # Errors
    /// [`BenchError`] on an unregistered image or an unsupported feature.
    pub fn front_paint(
        paint: &Paint,
        images: &HashMap<(ResourceHash, cherenkov_scene::ImageEncoding), cherenkov::ImageId>,
        front: &Front,
    ) -> Result<cherenkov::Paint, BenchError> {
        Ok(match paint {
            Paint::Transformed { paint, transform } => {
                front_paint(paint, images, front)?.transformed(*transform)
            }
            Paint::Mesh(mesh) => cherenkov::MeshGradient::new(
                mesh.columns(),
                mesh.rows(),
                mesh.points().to_vec(),
                mesh.colors().iter().map(working).collect(),
            )
            .interpolation(match mesh.interpolation_mode() {
                cherenkov_scene::MeshColorInterpolation::Linear => {
                    cherenkov::MeshColorInterpolation::Linear
                }
                cherenkov_scene::MeshColorInterpolation::Smoothstep => {
                    cherenkov::MeshColorInterpolation::Smoothstep
                }
            })
            .into(),
            Paint::Solid(c) => cherenkov::Paint::Solid(working(c)),
            Paint::Linear(g) => cherenkov::Paint::Linear(cherenkov::LinearGradient {
                start: g.start,
                end: g.end,
                stops: stops(&g.stops),
                extend: front_extend(g.extend),
                interpolation: interpolation(front, g.interpolation)?,
            }),
            Paint::Radial(g) => cherenkov::Paint::Radial(cherenkov::RadialGradient {
                start_center: g.center0,
                start_radius: g.r0,
                end_center: g.center1,
                end_radius: g.r1,
                stops: stops(&g.stops),
                extend: front_extend(g.extend),
                interpolation: interpolation(front, g.interpolation)?,
            }),
            Paint::Sweep(g) => cherenkov::Paint::Sweep(cherenkov::SweepGradient {
                center: g.center,
                start_angle: g.start_angle,
                end_angle: g.end_angle,
                stops: stops(&g.stops),
                extend: front_extend(g.extend),
                interpolation: interpolation(front, g.interpolation)?,
            }),
            Paint::Image(p) => cherenkov::Paint::Image(cherenkov::ImagePattern {
                image: *images
                    .get(&(p.image, p.encoding))
                    .ok_or(cherenkov_scene::SceneError::MissingResource(p.image))?,
                transform: p.transform,
                extend_x: front_extend(p.extend_x),
                extend_y: front_extend(p.extend_y),
                sampling: match p.sampling {
                    cherenkov_scene::Sampling::Nearest => cherenkov::Sampling::Nearest,
                    cherenkov_scene::Sampling::Bilinear => cherenkov::Sampling::Linear,
                },
            }),
        })
    }

    /// A scene shape → [`ShapeKind`].
    #[must_use]
    pub fn shape_kind(shape: &Shape, rule: cherenkov::FillRule) -> ShapeKind {
        match shape {
            Shape::Rect(r) => ShapeKind::Rect(*r),
            Shape::RoundedRect(r) => ShapeKind::RoundedRect(*r),
            Shape::Continuous(c) => ShapeKind::Continuous(
                cherenkov::ContinuousRect::new(c.rect, c.corner_radius).with_smoothing(c.smoothing),
            ),
            Shape::Circle(c) => ShapeKind::Circle(*c),
            Shape::Ellipse(e) => ShapeKind::Ellipse(*e),
            Shape::Line(l) => ShapeKind::Line(*l),
            Shape::Path { path } => {
                let cherenkov::ShapeData::Path { elements, .. } = cherenkov::ShapeData::of(path)
                else {
                    unreachable!()
                };
                ShapeKind::Path {
                    path: path.clone(),
                    data: cherenkov::ShapeData::Path { elements, rule },
                }
            }
        }
    }

    /// A scene draw → an [`Op`]; unreachable features still report unsupported
    /// rather than silently dropping.
    ///
    /// # Errors
    /// [`BenchError`] on an unregistered resource or an unsupported feature.
    pub fn op(
        draw: &Draw,
        fonts: &HashMap<(ResourceHash, u32), cherenkov::Font>,
        images: &HashMap<(ResourceHash, cherenkov_scene::ImageEncoding), cherenkov::ImageId>,
        blobs: &Blobs,
        front: &Front,
    ) -> Result<Op, BenchError> {
        Ok(match draw {
            Draw::Fill { shape, rule, paint } => Op::Fill {
                shape: shape_kind(shape, (front.fill_rule)(*rule)),
                rule: *rule,
                paint: front_paint(paint, images, front)?,
            },
            Draw::Stroke {
                shape,
                stroke,
                paint,
            } => Op::Stroke {
                shape: shape_kind(shape, cherenkov::FillRule::NonZero),
                stroke: super::stroke(stroke),
                paint: front_paint(paint, images, front)?,
            },
            Draw::Shadow {
                shape,
                blur_sigma,
                offset,
                color,
            } => Op::Shadow {
                shape: shape_kind(shape, cherenkov::FillRule::NonZero),
                shadow: cherenkov::Shadow::new(*blur_sigma, working(color))
                    .offset(kurbo::Vec2::new(offset[0], offset[1])),
            },
            Draw::Glyphs(run) => Op::Glyphs {
                run: glyph_run(run, fonts, blobs)?,
                paint: front_paint(&run.paint, images, front)?,
            },
            Draw::Image {
                image,
                encoding,
                dst,
                sampling,
            } => Op::Image {
                image: *images
                    .get(&(*image, *encoding))
                    .ok_or(cherenkov_scene::SceneError::MissingResource(*image))?,
                dst: *dst,
                sampling: match sampling {
                    cherenkov_scene::Sampling::Nearest => cherenkov::Sampling::Nearest,
                    cherenkov_scene::Sampling::Bilinear => cherenkov::Sampling::Linear,
                },
            },
        })
    }

    /// A scene group → one scoped [`Op::Group`] of member ops.
    ///
    /// # Errors
    /// [`BenchError`] on an unregistered resource or an unsupported feature.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "group opacity is f32 at the engine boundary"
    )]
    pub fn group_op(
        group: &cherenkov_scene::Group,
        fonts: &HashMap<(ResourceHash, u32), cherenkov::Font>,
        images: &HashMap<(ResourceHash, cherenkov_scene::ImageEncoding), cherenkov::ImageId>,
        blobs: &Blobs,
        front: &Front,
    ) -> Result<Op, BenchError> {
        let mut ops = Vec::with_capacity(group.items.len());
        for item in &group.items {
            ops.push(match item {
                cherenkov_scene::GroupItem::Draw(d) => op(d, fonts, images, blobs, front)?,
                cherenkov_scene::GroupItem::Group(inner) => {
                    group_op(inner, fonts, images, blobs, front)?
                }
            });
        }
        Ok(Op::Group {
            group: cherenkov::Group::new()
                .opacity(group.opacity as f32)
                .blend(engine_blend(group.blend))
                .blend_space(match group.blend_space {
                    cherenkov_scene::BlendSpace::Linear => cherenkov::BlendSpace::Linear,
                    cherenkov_scene::BlendSpace::SrgbEncoded => cherenkov::BlendSpace::SrgbEncoded,
                }),
            ops,
        })
    }

    /// A text layer's source → an [`Op::Text`]: the source shaped with
    /// parley, its paints lowered like any draw's, and each run's font
    /// resolved to the engine font registered for its blob.
    ///
    /// # Errors
    /// [`BenchError`] on a missing blob, an unregistered font or image, an
    /// unsupported paint, or a layout the engine's text adapter rejects.
    pub fn text_op(
        source: &cherenkov_scene::TextSource,
        fonts: &HashMap<(ResourceHash, u32), cherenkov::Font>,
        images: &HashMap<(ResourceHash, cherenkov_scene::ImageEncoding), cherenkov::ImageId>,
        blobs: &Blobs,
        front: &Front,
    ) -> Result<Op, BenchError> {
        let cherenkov_scene::ShapedText { layout, resources } = source.shape(
            |hash| blobs.get(hash).map(Vec::as_slice),
            |paint| front_paint(paint, images, front),
        )?;
        let fonts_of = |data: &cherenkov::parley::FontData| {
            resources
                .resource(data)
                .and_then(|hash| fonts.get(&(hash, data.index)))
                .cloned()
                .ok_or_else(|| {
                    cherenkov::ResourceError::Font(format!(
                        "no registered font for blob {} index {}",
                        data.data.id(),
                        data.index
                    ))
                })
        };
        let layout = cherenkov::TextLayout::new(layout, fonts_of)
            .map_err(|e| BenchError::Engine(format!("{}: text layout: {e}", front.engine)))?;
        Ok(Op::Text {
            layout,
            origin: source.origin,
        })
    }

    /// A scene glyph run → a front-end run with the registered font and the
    /// resolved `F2Dot14` coordinates.
    ///
    /// # Errors
    /// [`BenchError`] when the run's font is unregistered.
    pub fn glyph_run(
        run: &cherenkov_scene::GlyphRun,
        fonts: &HashMap<(ResourceHash, u32), cherenkov::Font>,
        blobs: &Blobs,
    ) -> Result<cherenkov::GlyphRun, BenchError> {
        let font = fonts
            .get(&(run.font, run.font_index))
            .map(cherenkov::Font::id)
            .ok_or(cherenkov_scene::SceneError::MissingResource(run.font))?;
        let coords = blobs
            .get(&run.font)
            .map_or_else(Vec::new, |b| coord_bits(b, &run.normalized_coords));
        Ok(cherenkov::GlyphRun {
            font,
            size: run.size,
            coords: coords.into(),
            glyphs: run
                .glyphs
                .iter()
                .map(|g| cherenkov::Glyph {
                    id: g.id,
                    x: g.x,
                    y: g.y,
                    transform: g.transform,
                })
                .collect::<Vec<_>>()
                .into(),
            style: run
                .stroke
                .as_ref()
                .map_or(cherenkov::GlyphStyle::Fill, |stroke| {
                    cherenkov::GlyphStyle::Stroke(stroke.into())
                }),
        })
    }
}

#[cfg(any(feature = "cherenkov", feature = "cherenkov-cpu"))]
pub(crate) use front::*;

#[cfg(test)]
mod tests {
    use super::*;

    /// A variable font (`wdth,wght`) with only `wght` specified: the
    /// omitted `wdth` axis must resolve to normalized `0` (the font
    /// default), not the axis's user-space default — a value like `75`
    /// would saturate `F2Dot14` around `±2` and render the axis at its
    /// extreme.
    #[test]
    fn coord_bits_omitted_axis_is_normalized_zero() {
        let font = std::fs::read("../scenes/fonts/NotoSans.ttf").expect("test font");
        let bits = coord_bits(
            &font,
            &[NormalizedCoord {
                tag: "wght".into(),
                value: 1.0,
            }],
        );
        assert_eq!(bits.len(), 2, "wdth,wght font should have two axes");
        assert!(
            bits.contains(&0),
            "omitted wdth axis should produce bits 0: {bits:?}"
        );
        let wght = skrifa::raw::types::F2Dot14::from_f32(1.0).to_bits();
        assert!(bits.contains(&wght), "wght=1.0 bits missing: {bits:?}");
    }
}
