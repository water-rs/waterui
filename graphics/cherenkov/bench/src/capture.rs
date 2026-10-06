//! Converts a captured `cherenkov` frame — a [`DisplayList`] plus the
//! blobs its registrations produced — into a [`cherenkov_scene::Scene`]
//! (#211).
//!
//! The capture side records with a `StaticRecorder` and keeps the engine
//! id of every font and image the recording names. Scopes map to scene
//! layers (`BeginClip`, `BeginTransform`, `Picture`) or groups
//! (`BeginGroup`); a group whose body carries layers is hoisted to a
//! layer, which requires the default blend space. Engine-only features —
//! shader paints, live group filters, shadow spread, non-uniform
//! continuous-corner radii — fail loudly rather than silently dropping.

use std::collections::BTreeSet;
use std::sync::Arc;

use cherenkov::{
    BlendMode as EngineBlend, BlendSpace as EngineBlendSpace, Command, DisplayList,
    Extend as EngineExtend, FillRule, GlyphRun as EngineRun, GlyphStyle, Interpolation,
    Paint as EnginePaint, Sampling as EngineSampling, ShapeData, Stroke,
};
use cherenkov_scene::{
    BlendMode, BlendSpace, Color, ColorSpace, ContinuousRect, Draw, Extend, Glyph, GlyphRun,
    GradientStop, Group, GroupItem, ImageEncoding, ImagePaint, Item, Layer, LinearGradient,
    MeshColorInterpolation, MeshGradient, NormalizedCoord, Paint, RadialGradient, ResourceHash,
    Sampling, Scene, Shape, StrokeStyle, SweepGradient, WorkingSpace,
};
use kurbo::{Affine, BezPath, RoundedRectRadii};
use skrifa::MetadataProvider;

/// A font a captured glyph run names, keyed by its engine `FontId` raw
/// value.
#[derive(Debug)]
pub struct CapturedFont {
    /// `FontId::raw()`.
    pub id: u64,
    /// The registered font data.
    pub data: Arc<[u8]>,
    /// The index inside a collection.
    pub index: u32,
}

/// An image a captured `Image` command or `ImagePaint` names, keyed by its
/// engine `ImageId` raw value. `data` is the exact blob stored in
/// `resources/` and decoded by `encoding`.
#[derive(Debug)]
pub struct CapturedImage {
    /// `ImageId::raw()`.
    pub id: u64,
    /// The encoded blob.
    pub data: Arc<[u8]>,
    /// How `data` decodes.
    pub encoding: ImageEncoding,
}

/// One recorded frame plus every resource it names.
#[derive(Debug)]
pub struct Capture {
    /// Frame size in physical pixels.
    pub width: u32,
    /// Frame size in physical pixels.
    pub height: u32,
    /// The colour the frame is cleared to before drawing.
    pub clear: Color,
    /// The recorded commands.
    pub list: DisplayList,
    /// Fonts registered while recording.
    pub fonts: Vec<CapturedFont>,
    /// Images registered while recording.
    pub images: Vec<CapturedImage>,
}

/// A construct the capture cannot express in the scene format.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CaptureError {
    /// The content uses a capability with no scene equivalent.
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
    /// A draw names a font or image the capture did not carry.
    #[error("resource id {0} is not in the capture")]
    MissingResource(u64),
    /// A glyph run carries variation coordinates on a font whose `fvar`
    /// axes cannot be read.
    #[error("font resource id {0} has no readable variation axes")]
    BadFont(u64),
}

/// A resource blob plus the hash the converted scene names it by.
#[derive(Debug)]
pub struct Resource {
    /// The content hash under `resources/`.
    pub hash: ResourceHash,
    /// The blob to store as `hash.file_name()`.
    pub data: Arc<[u8]>,
}

/// Converts the frame into a scene and the blobs its `resources/`
/// directory needs.
///
/// # Errors
/// Any [`CaptureError`]: the frame is refused rather than lossy.
///
/// # Panics
/// When the display list carries more than `u32::MAX` commands — the
/// engine's `end` indices are already `u32`, so no recorded frame can.
pub fn convert(capture: &Capture) -> Result<(Scene, Vec<Resource>), CaptureError> {
    let fonts: Vec<(u64, &CapturedFont, ResourceHash)> = capture
        .fonts
        .iter()
        .map(|f| (f.id, f, ResourceHash::of(&f.data)))
        .collect();
    let images: Vec<(u64, ResourceHash, ImageEncoding)> = capture
        .images
        .iter()
        .map(|i| (i.id, ResourceHash::of(&i.data), i.encoding))
        .collect();
    let resources = capture
        .fonts
        .iter()
        .map(|f| Resource {
            hash: ResourceHash::of(&f.data),
            data: f.data.clone(),
        })
        .chain(capture.images.iter().map(|i| Resource {
            hash: ResourceHash::of(&i.data),
            data: i.data.clone(),
        }))
        .collect();
    let state = State { fonts, images };
    let mut scene = Scene {
        width: capture.width,
        height: capture.height,
        working_space: WorkingSpace::LinearDisplayP3,
        clear: capture.clear,
        present_headroom: 1.0,
        features: BTreeSet::default(),
        backdrop_groups: Vec::new(),
        root: Layer {
            items: state.items(
                &capture.list,
                0,
                u32::try_from(capture.list.commands().len()).expect("command count fits u32"),
            )?,
            ..Default::default()
        },
    };
    scene.compute_features();
    Ok((scene, resources))
}

/// Per-frame lookup state: engine ids to resource hashes and blobs.
struct State<'a> {
    fonts: Vec<(u64, &'a CapturedFont, ResourceHash)>,
    images: Vec<(u64, ResourceHash, ImageEncoding)>,
}

impl State<'_> {
    /// Converts `list` commands `[start, end)` — `end` is the index of a
    /// scope's matching `End`, or the command count for the root walk.
    #[expect(
        clippy::too_many_lines,
        reason = "one match arm per display-list command"
    )]
    fn items(&self, list: &DisplayList, start: u32, end: u32) -> Result<Vec<Item>, CaptureError> {
        let mut items = Vec::new();
        let mut i = start;
        while i < end {
            match &list.commands()[i as usize] {
                Command::Fill { shape, paint } => {
                    let (shape, rule) = Self::shape(shape)?;
                    items.push(Item::Draw(Draw::Fill {
                        shape,
                        rule: fill_rule(rule),
                        paint: self.paint(paint)?,
                    }));
                    i += 1;
                }
                Command::Stroke {
                    shape,
                    stroke,
                    paint,
                } => {
                    items.push(Item::Draw(Draw::Stroke {
                        shape: Self::shape(shape)?.0,
                        stroke: stroke_style(stroke),
                        paint: self.paint(paint)?,
                    }));
                    i += 1;
                }
                Command::Shadow { shape, shadow } => {
                    if shadow.spread != 0.0 {
                        return Err(CaptureError::Unsupported("shadow spread"));
                    }
                    items.push(Item::Draw(Draw::Shadow {
                        shape: Self::shape(shape)?.0,
                        blur_sigma: shadow.sigma,
                        offset: [shadow.offset.x, shadow.offset.y],
                        color: color(&shadow.color),
                    }));
                    i += 1;
                }
                Command::Glyphs { run, paint } => {
                    items.push(Item::Draw(Draw::Glyphs(self.glyph_run(run, paint)?)));
                    i += 1;
                }
                Command::Image {
                    image,
                    dst,
                    sampling: sample,
                } => {
                    let (_, hash, encoding) = self
                        .images
                        .iter()
                        .find(|(id, ..)| *id == image.raw())
                        .ok_or_else(|| CaptureError::MissingResource(image.raw()))?;
                    items.push(Item::Draw(Draw::Image {
                        image: *hash,
                        encoding: *encoding,
                        dst: *dst,
                        sampling: sampling(*sample),
                    }));
                    i += 1;
                }
                Command::Text { .. } => {
                    return Err(CaptureError::Unsupported(
                        "a text layout the target drew itself; scenes carry glyph runs",
                    ));
                }
                Command::Picture { picture, transform } => {
                    let list = picture.display_list();
                    let body = self.items(
                        list,
                        0,
                        u32::try_from(list.commands().len()).expect("command count fits u32"),
                    )?;
                    if *transform == Affine::IDENTITY {
                        items.extend(body);
                    } else {
                        items.push(Item::Layer(Layer {
                            transform: *transform,
                            items: body,
                            ..Default::default()
                        }));
                    }
                    i += 1;
                }
                Command::BeginClip { shape, end } => {
                    items.push(Item::Layer(Layer {
                        clip: Some(Self::shape(shape)?.0),
                        items: self.items(list, i + 1, *end)?,
                        ..Default::default()
                    }));
                    i = *end + 1;
                }
                Command::BeginTransform { transform, end } => {
                    items.push(Item::Layer(Layer {
                        transform: *transform,
                        items: self.items(list, i + 1, *end)?,
                        ..Default::default()
                    }));
                    i = *end + 1;
                }
                Command::BeginGroup { group, end } => {
                    if group.filter.is_some() {
                        return Err(CaptureError::Unsupported("live group filter"));
                    }
                    let body = self.items(list, i + 1, *end)?;
                    if let Some(draws) = draws_only(&body) {
                        items.push(Item::Group(Group {
                            items: draws,
                            opacity: f64::from(group.opacity),
                            blend: blend(group.blend),
                            blend_space: blend_space(group.blend_space),
                        }));
                    } else if group.blend_space == EngineBlendSpace::Linear {
                        // A group carrying layers cannot sit in `GroupItem`;
                        // a layer composites identically — same isolation,
                        // opacity and blend — at the default blend space.
                        items.push(Item::Layer(Layer {
                            opacity: f64::from(group.opacity),
                            blend: blend(group.blend),
                            items: body,
                            ..Default::default()
                        }));
                    } else {
                        return Err(CaptureError::Unsupported(
                            "a group with layers in a non-linear blend space",
                        ));
                    }
                    i = *end + 1;
                }
                Command::End => return Err(CaptureError::Unsupported("stray End")),
            }
        }
        Ok(items)
    }

    /// A shape plus its fill rule (the rule travels on `Draw::Fill`, not
    /// on `Shape`).
    fn shape(shape: &ShapeData) -> Result<(Shape, FillRule), CaptureError> {
        Ok(match shape {
            ShapeData::Rect(rect) => (Shape::Rect(*rect), FillRule::NonZero),
            ShapeData::RoundedRect(rect) => (Shape::RoundedRect(*rect), FillRule::NonZero),
            ShapeData::Continuous(rect) => (
                Shape::Continuous(ContinuousRect {
                    rect: rect.rect,
                    corner_radius: uniform_radius(rect.radii)?,
                    smoothing: rect.smoothing,
                }),
                FillRule::NonZero,
            ),
            ShapeData::Circle(circle) => (Shape::Circle(*circle), FillRule::NonZero),
            ShapeData::Ellipse(ellipse) => (Shape::Ellipse(*ellipse), FillRule::NonZero),
            ShapeData::Line(line) => (Shape::Line(*line), FillRule::NonZero),
            ShapeData::Path { elements, rule } => (
                Shape::Path {
                    path: BezPath::from_vec(elements.to_vec()),
                },
                *rule,
            ),
        })
    }

    fn paint(&self, paint: &EnginePaint) -> Result<Paint, CaptureError> {
        Ok(match paint {
            EnginePaint::Solid(c) => Paint::Solid(color(c)),
            EnginePaint::Linear(g) => Paint::Linear(LinearGradient {
                start: g.start,
                end: g.end,
                stops: stops(&g.stops),
                extend: extend(g.extend),
                interpolation: interpolation(g.interpolation),
            }),
            EnginePaint::Radial(g) => Paint::Radial(RadialGradient {
                center0: g.start_center,
                r0: g.start_radius,
                center1: g.end_center,
                r1: g.end_radius,
                stops: stops(&g.stops),
                extend: extend(g.extend),
                interpolation: interpolation(g.interpolation),
            }),
            EnginePaint::Sweep(g) => Paint::Sweep(SweepGradient {
                center: g.center,
                start_angle: g.start_angle,
                end_angle: g.end_angle,
                stops: stops(&g.stops),
                extend: extend(g.extend),
                interpolation: interpolation(g.interpolation),
            }),
            EnginePaint::Mesh(m) => Paint::Mesh(
                MeshGradient::new(
                    m.columns(),
                    m.rows(),
                    m.points().to_vec(),
                    m.colors().iter().map(color).collect(),
                )
                .interpolation(match m.interpolation_mode() {
                    cherenkov::MeshColorInterpolation::Linear => MeshColorInterpolation::Linear,
                    cherenkov::MeshColorInterpolation::Smoothstep => {
                        MeshColorInterpolation::Smoothstep
                    }
                }),
            ),
            EnginePaint::Image(p) => {
                let (_, hash, encoding) = self
                    .images
                    .iter()
                    .find(|(id, ..)| *id == p.image.raw())
                    .ok_or_else(|| CaptureError::MissingResource(p.image.raw()))?;
                Paint::Image(ImagePaint {
                    image: *hash,
                    encoding: *encoding,
                    transform: p.transform,
                    extend_x: extend(p.extend_x),
                    extend_y: extend(p.extend_y),
                    sampling: sampling(p.sampling),
                })
            }
            EnginePaint::Shader(_) => return Err(CaptureError::Unsupported("shader paint")),
            EnginePaint::Transformed(t) => Paint::Transformed {
                paint: Box::new(self.paint(&t.paint)?),
                transform: t.transform,
            },
        })
    }

    fn glyph_run(&self, run: &EngineRun, paint: &EnginePaint) -> Result<GlyphRun, CaptureError> {
        let (_, font, hash) = self
            .fonts
            .iter()
            .find(|(id, ..)| *id == run.font.raw())
            .ok_or_else(|| CaptureError::MissingResource(run.font.raw()))?;
        Ok(GlyphRun {
            font: *hash,
            font_index: font.index,
            size: run.size,
            normalized_coords: Self::coords(run, font)?,
            glyphs: run
                .glyphs
                .iter()
                .map(|g| Glyph {
                    id: g.id,
                    x: g.x,
                    y: g.y,
                    transform: g.transform,
                })
                .collect(),
            stroke: match &run.style {
                GlyphStyle::Fill => None,
                GlyphStyle::Stroke(stroke) => Some(stroke_style(stroke)),
            },
            paint: self.paint(paint)?,
        })
    }

    /// `F2Dot14` bits in the font's axis order to named normalized
    /// coordinates.
    fn coords(run: &EngineRun, font: &CapturedFont) -> Result<Vec<NormalizedCoord>, CaptureError> {
        if run.coords.is_empty() {
            return Ok(Vec::new());
        }
        let parsed = skrifa::FontRef::new(&font.data[..])
            .map_err(|_| CaptureError::BadFont(run.font.raw()))?;
        let axes: Vec<String> = parsed.axes().iter().map(|a| a.tag().to_string()).collect();
        if axes.len() != run.coords.len() {
            return Err(CaptureError::BadFont(run.font.raw()));
        }
        Ok(axes
            .into_iter()
            .zip(run.coords.iter())
            .map(|(tag, bits)| NormalizedCoord {
                tag,
                value: skrifa::raw::types::F2Dot14::from_bits(*bits).to_f32(),
            })
            .collect())
    }
}

/// `Some(draws)` when every item is a draw — the only form a `GroupItem`
/// can carry.
fn draws_only(items: &[Item]) -> Option<Vec<GroupItem>> {
    items
        .iter()
        .all(|item| matches!(item, Item::Draw(_)))
        .then(|| {
            items
                .iter()
                .map(|item| {
                    let Item::Draw(draw) = item else {
                        unreachable!("draws_only checked every variant");
                    };
                    GroupItem::Draw(draw.clone())
                })
                .collect()
        })
}

/// A continuous corner needs one radius for all four corners; per-corner
/// radii have no scene equivalent.
#[expect(
    clippy::float_cmp,
    reason = "the radii must be exactly equal — approximate equality would hide an unsupported case"
)]
fn uniform_radius(radii: RoundedRectRadii) -> Result<f64, CaptureError> {
    let r = radii.top_left;
    if r == radii.top_right && r == radii.bottom_right && r == radii.bottom_left {
        Ok(r)
    } else {
        Err(CaptureError::Unsupported(
            "continuous rect with non-uniform radii",
        ))
    }
}

/// Working colours are linear Display P3.
const fn color(color: &cherenkov::WorkingColor) -> Color {
    Color {
        space: ColorSpace::LinearP3,
        components: color.components,
    }
}

fn stops(stops: &[cherenkov::ColorStop]) -> Vec<GradientStop> {
    stops
        .iter()
        .map(|s| GradientStop {
            offset: s.offset,
            color: color(&s.color),
        })
        .collect()
}

const fn extend(extend: EngineExtend) -> Extend {
    match extend {
        EngineExtend::Pad => Extend::Pad,
        EngineExtend::Repeat => Extend::Repeat,
        EngineExtend::Reflect => Extend::Reflect,
        EngineExtend::None => Extend::None,
    }
}

const fn sampling(sampling: EngineSampling) -> Sampling {
    match sampling {
        EngineSampling::Nearest => Sampling::Nearest,
        EngineSampling::Linear => Sampling::Bilinear,
    }
}

const fn fill_rule(rule: FillRule) -> cherenkov_scene::FillRule {
    match rule {
        FillRule::NonZero => cherenkov_scene::FillRule::NonZero,
        FillRule::EvenOdd => cherenkov_scene::FillRule::EvenOdd,
    }
}

const fn interpolation(interpolation: Interpolation) -> ColorSpace {
    match interpolation {
        Interpolation::Working => ColorSpace::LinearP3,
        Interpolation::SrgbEncoded => ColorSpace::Srgb,
    }
}

const fn blend(blend: EngineBlend) -> BlendMode {
    match blend {
        EngineBlend::Normal => BlendMode::Normal,
        EngineBlend::Multiply => BlendMode::Multiply,
        EngineBlend::Screen => BlendMode::Screen,
        EngineBlend::Overlay => BlendMode::Overlay,
        EngineBlend::Darken => BlendMode::Darken,
        EngineBlend::Lighten => BlendMode::Lighten,
        EngineBlend::ColorDodge => BlendMode::ColorDodge,
        EngineBlend::ColorBurn => BlendMode::ColorBurn,
        EngineBlend::HardLight => BlendMode::HardLight,
        EngineBlend::SoftLight => BlendMode::SoftLight,
        EngineBlend::Difference => BlendMode::Difference,
        EngineBlend::Exclusion => BlendMode::Exclusion,
        EngineBlend::Hue => BlendMode::Hue,
        EngineBlend::Saturation => BlendMode::Saturation,
        EngineBlend::Color => BlendMode::Color,
        EngineBlend::Luminosity => BlendMode::Luminosity,
        EngineBlend::Clear => BlendMode::Clear,
        EngineBlend::Src => BlendMode::Src,
        EngineBlend::Dst => BlendMode::Dst,
        EngineBlend::DestOver => BlendMode::DestOver,
        EngineBlend::SrcIn => BlendMode::SrcIn,
        EngineBlend::DestIn => BlendMode::DestIn,
        EngineBlend::SrcOut => BlendMode::SrcOut,
        EngineBlend::DestOut => BlendMode::DestOut,
        EngineBlend::SrcAtop => BlendMode::SrcAtop,
        EngineBlend::DestAtop => BlendMode::DestAtop,
        EngineBlend::Xor => BlendMode::Xor,
        EngineBlend::PlusLighter => BlendMode::PlusLighter,
    }
}

const fn blend_space(space: EngineBlendSpace) -> BlendSpace {
    match space {
        EngineBlendSpace::Linear => BlendSpace::Linear,
        EngineBlendSpace::SrgbEncoded => BlendSpace::SrgbEncoded,
    }
}

fn stroke_style(stroke: &Stroke) -> StrokeStyle {
    StrokeStyle {
        width: stroke.width,
        join: stroke.join,
        miter_limit: stroke.miter_limit,
        start_cap: stroke.start_cap,
        end_cap: stroke.end_cap,
        dash_pattern: stroke.dash_pattern.to_vec(),
        dash_offset: stroke.dash_offset,
    }
}
