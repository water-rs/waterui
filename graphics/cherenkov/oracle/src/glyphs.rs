//! Glyph rendering via `skrifa`: **unhinted** outlines at the exact size.
//!
//! Outlines are expanded into scene [`Item`]s the renderer walks like
//! authored content — `COLRv1` glyphs become nested layers (clips, blend
//! modes, gradient fills), plain glyphs a single fill.
//!
//! Glyph space is font units, y-up; each glyph is placed by
//! `translate(x, y) * scale_non_uniform(size/upem, -size/upem)` so the scene
//! position `x, y` is the glyph's origin on the baseline in y-down scene
//! coordinates.
//!
//! `COLRv1` brush transforms are exact: gradient geometry stays in
//! brush-local coordinates under a [`Paint::Transformed`] map, so skewed
//! and non-uniformly scaled radial/sweep brushes are evaluated in their
//! own space. Font gradients are interpolated in sRGB per the `COLRv1`
//! spec's use of CSS images semantics; [`ColorSpace::Srgb`] is used as the
//! interpolation space.

use crate::color::{linear_srgb_to_linear_p3, srgb_decode};
use cherenkov_scene::{
    BlendMode, Color, ColorSpace, Draw, Extend, FillRule, GlyphRun, GradientStop, ImageEncoding,
    Item, Layer, LinearGradient, Paint, RadialGradient, ResourceHash, Sampling, Shape,
    SweepGradient,
};
use kurbo::{Affine, BezPath, Point, Rect, Shape as _};
use read_fonts::types::BoundingBox;
use skrifa::{
    GlyphId, MetadataProvider,
    bitmap::{BitmapData, BitmapFormat, BitmapGlyph, BitmapStrikes, Origin},
    color::{Brush, ColorPainter, ColorStop},
    instance::{LocationRef, NormalizedCoord as F2Dot14Coord, Size},
    outline::{DrawSettings, OutlinePen},
    raw::TableProvider,
};

use crate::resources::Resources;

/// Errors from glyph loading or painting.
#[derive(Debug)]
pub enum GlyphError {
    /// The font blob failed to parse.
    Font(String),
    /// A glyph id is out of range for `u16`.
    GlyphId(u32),
    /// The font has no outline for this glyph.
    NoOutline(u16),
    /// A `COLRv1` paint graph failed.
    Paint(String),
    /// A composite mode outside the W3C-16 set was encountered.
    UnsupportedCompositeMode(String),
    /// A per-glyph transform is non-finite or non-invertible.
    Transform,
    /// A bitmap-font feature or graphic format is unsupported.
    UnsupportedBitmap(String),
}

impl std::fmt::Display for GlyphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Font(e) => write!(f, "font parse error: {e}"),
            Self::GlyphId(id) => write!(f, "glyph id {id} does not fit in u16"),
            Self::NoOutline(id) => write!(f, "glyph {id} has no outline"),
            Self::Paint(e) => write!(f, "COLR paint error: {e}"),
            Self::UnsupportedCompositeMode(m) => write!(f, "unsupported COLR composite mode {m}"),
            Self::Transform => write!(f, "glyph transform must be finite and invertible"),
            Self::UnsupportedBitmap(feature) => write!(f, "unsupported bitmap font {feature}"),
        }
    }
}

impl std::error::Error for GlyphError {}

/// Collects path commands into a [`BezPath`].
struct BezPen(BezPath);

impl OutlinePen for BezPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((f64::from(x), f64::from(y)));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((f64::from(x), f64::from(y)));
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.0
            .quad_to((f64::from(cx), f64::from(cy)), (f64::from(x), f64::from(y)));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to(
            (f64::from(cx0), f64::from(cy0)),
            (f64::from(cx1), f64::from(cy1)),
            (f64::from(x), f64::from(y)),
        );
    }
    fn close(&mut self) {
        self.0.close_path();
    }
}

/// A node of the paint tree a `COLRv1` painter builds, in **font units**.
enum Node {
    /// Fill `shape` (already transform-applied in font space) with `paint`
    /// (gradient coordinates in font space). `shape == None` fills the
    /// enclosing clip region — emitted as a whole-canvas fill, bounded by
    /// whatever clips enclose it.
    Fill {
        shape: Option<BezPath>,
        paint: Paint,
    },
    /// A group: optional clip + blend mode applied to `children`.
    Group {
        clip: Option<BezPath>,
        blend: BlendMode,
        /// Group opacity: a foreground brush's COLR `alpha` when the run
        /// paint carries no alpha channel.
        opacity: f32,
        children: Vec<Self>,
    },
}

fn composite_to_blend(mode: skrifa::color::CompositeMode) -> Result<BlendMode, GlyphError> {
    use skrifa::color::CompositeMode as Cm;
    Ok(match mode {
        Cm::Clear => BlendMode::Clear,
        Cm::Src => BlendMode::Src,
        Cm::Dest => BlendMode::Dst,
        Cm::SrcOver => BlendMode::Normal,
        Cm::DestOver => BlendMode::DestOver,
        Cm::SrcIn => BlendMode::SrcIn,
        Cm::DestIn => BlendMode::DestIn,
        Cm::SrcOut => BlendMode::SrcOut,
        Cm::DestOut => BlendMode::DestOut,
        Cm::SrcAtop => BlendMode::SrcAtop,
        Cm::DestAtop => BlendMode::DestAtop,
        Cm::Xor => BlendMode::Xor,
        // COLR `Plus` is the spec's "plus lighter" additive mode.
        Cm::Plus => BlendMode::PlusLighter,
        Cm::Screen => BlendMode::Screen,
        Cm::Overlay => BlendMode::Overlay,
        Cm::Darken => BlendMode::Darken,
        Cm::Lighten => BlendMode::Lighten,
        Cm::ColorDodge => BlendMode::ColorDodge,
        Cm::ColorBurn => BlendMode::ColorBurn,
        Cm::HardLight => BlendMode::HardLight,
        Cm::SoftLight => BlendMode::SoftLight,
        Cm::Difference => BlendMode::Difference,
        Cm::Exclusion => BlendMode::Exclusion,
        Cm::Multiply => BlendMode::Multiply,
        Cm::HslHue => BlendMode::Hue,
        Cm::HslSaturation => BlendMode::Saturation,
        Cm::HslColor => BlendMode::Color,
        Cm::HslLuminosity => BlendMode::Luminosity,
        other => return Err(GlyphError::UnsupportedCompositeMode(format!("{other:?}"))),
    })
}

const fn extend(e: skrifa::color::Extend) -> Extend {
    match e {
        skrifa::color::Extend::Repeat => Extend::Repeat,
        skrifa::color::Extend::Reflect => Extend::Reflect,
        // `Pad` and anything unrecognised pad the edge stops.
        _ => Extend::Pad,
    }
}

/// The `COLRv1` painter: keeps a transform stack (font space), a container
/// stack for clips and composite layers, and emits [`Node`]s.
struct ColrPainter<'a> {
    font: &'a skrifa::FontRef<'a>,
    coords: &'a [F2Dot14Coord],
    palette: Vec<skrifa::color::Color>,
    /// The run's own paint — the COLR "foreground" brush
    /// (`palette_index == 0xFFFF`) for solid brushes.
    foreground: &'a Paint,
    /// `foreground`'s colour, respecting its declared colour space. For a
    /// non-solid run paint — which a gradient stop colour cannot express —
    /// this falls back to opaque sRGB black; solid foreground brushes still
    /// emit the full run paint.
    foreground_color: Color,
    /// Canvas rect in font units — what a `fill` with no glyph clip covers.
    fill_rect_font: BezPath,
    tf: Vec<Affine>,
    containers: Vec<(Option<BezPath>, BlendMode, Vec<Node>)>,
    top: Vec<Node>,
    /// First failure recorded by a `ColorPainter` callback (the trait's
    /// methods cannot return `Result`); checked after `paint()` returns.
    err: Option<GlyphError>,
}

impl ColrPainter<'_> {
    fn cur(&self) -> Affine {
        *self.tf.last().unwrap_or(&Affine::IDENTITY)
    }

    fn palette_color(&self, index: u16, alpha: f32) -> Color {
        if index == 0xFFFF || usize::from(index) >= self.palette.len() {
            // The foreground keeps the run paint's declared colour space.
            let mut c = self.foreground_color;
            c.components[3] *= alpha;
            return c;
        }
        let c = self.palette[usize::from(index)];
        Color {
            space: ColorSpace::Srgb,
            components: [
                f32::from(c.red) / 255.0,
                f32::from(c.green) / 255.0,
                f32::from(c.blue) / 255.0,
                f32::from(c.alpha) / 255.0 * alpha,
            ],
        }
    }

    fn stops(&self, stops: &[ColorStop]) -> Vec<GradientStop> {
        stops
            .iter()
            .map(|s| GradientStop {
                offset: s.offset,
                color: self.palette_color(s.palette_index, s.alpha),
            })
            .collect()
    }

    /// Resolve a COLR brush into a scene [`Paint`]. Gradient geometry
    /// stays in brush-local font units under a paint transform `tf`, so
    /// non-similarity brushes are exact.
    fn brush_paint(&self, brush: &Brush<'_>, tf: Affine) -> Paint {
        match brush {
            Brush::Solid {
                palette_index,
                alpha,
            } => {
                if *palette_index == 0xFFFF {
                    // The foreground brush is the run's own paint — it may
                    // be a gradient or image, not just a solid colour. The
                    // COLR `alpha` applies to it as a paint opacity.
                    let mut paint = self.foreground.clone();
                    paint_opacity(&mut paint, *alpha);
                    paint
                } else {
                    Paint::Solid(self.palette_color(*palette_index, *alpha))
                }
            }
            Brush::LinearGradient {
                p0,
                p1,
                color_stops,
                extend: e,
            } => transformed(
                Paint::Linear(LinearGradient {
                    start: Point::new(f64::from(p0.x), f64::from(p0.y)),
                    end: Point::new(f64::from(p1.x), f64::from(p1.y)),
                    stops: self.stops(color_stops),
                    extend: extend(*e),
                    interpolation: ColorSpace::Srgb,
                }),
                tf,
            ),
            Brush::RadialGradient {
                c0,
                r0,
                c1,
                r1,
                color_stops,
                extend: e,
            } => transformed(
                Paint::Radial(RadialGradient {
                    center0: Point::new(f64::from(c0.x), f64::from(c0.y)),
                    r0: f64::from(*r0),
                    center1: Point::new(f64::from(c1.x), f64::from(c1.y)),
                    r1: f64::from(*r1),
                    stops: self.stops(color_stops),
                    extend: extend(*e),
                    interpolation: ColorSpace::Srgb,
                }),
                tf,
            ),
            Brush::SweepGradient {
                c0,
                start_angle,
                end_angle,
                color_stops,
                extend: e,
            } => transformed(
                Paint::Sweep(SweepGradient {
                    center: Point::new(f64::from(c0.x), f64::from(c0.y)),
                    // skrifa hands degrees, interpreted clockwise in y-up font
                    // space; after the y-flip into y-down scene space the same
                    // angles read clockwise on screen, which is our convention.
                    start_angle: f64::from(*start_angle).to_radians(),
                    end_angle: f64::from(*end_angle).to_radians(),
                    stops: self.stops(color_stops),
                    extend: extend(*e),
                    interpolation: ColorSpace::Srgb,
                }),
                tf,
            ),
        }
    }

    fn glyph_path(&self, glyph_id: GlyphId) -> Result<BezPath, GlyphError> {
        let id = u16::try_from(glyph_id.to_u32()).unwrap_or(0);
        let glyph = self
            .font
            .outline_glyphs()
            .get(glyph_id)
            .ok_or(GlyphError::NoOutline(id))?;
        let mut pen = BezPen(BezPath::new());
        glyph
            .draw(
                DrawSettings::unhinted(Size::unscaled(), LocationRef::new(self.coords)),
                &mut pen,
            )
            .map_err(|e| GlyphError::Font(e.to_string()))?;
        Ok(pen.0)
    }
}

impl ColorPainter for ColrPainter<'_> {
    fn push_transform(&mut self, transform: skrifa::color::Transform) {
        let c = transform;
        let affine = Affine::new([
            f64::from(c.xx),
            f64::from(c.yx),
            f64::from(c.xy),
            f64::from(c.yy),
            f64::from(c.dx),
            f64::from(c.dy),
        ]);
        self.tf.push(self.cur() * affine);
    }

    fn pop_transform(&mut self) {
        self.tf.pop();
    }

    fn push_clip_glyph(&mut self, glyph_id: GlyphId) {
        if self.err.is_some() {
            return;
        }
        let path = match self.glyph_path(glyph_id).map(|p| self.cur() * p) {
            Ok(p) => p,
            Err(e) => {
                self.err = Some(e);
                return;
            }
        };
        self.containers
            .push((Some(path), BlendMode::Normal, std::mem::take(&mut self.top)));
    }

    fn push_clip_box(&mut self, clip_box: BoundingBox<f32>) {
        let rect = Rect::new(
            f64::from(clip_box.x_min),
            f64::from(clip_box.y_min),
            f64::from(clip_box.x_max),
            f64::from(clip_box.y_max),
        );
        let path = self.cur() * rect.to_path(1e-9);
        self.containers
            .push((Some(path), BlendMode::Normal, std::mem::take(&mut self.top)));
    }

    fn pop_clip(&mut self) {
        if let Some((clip, blend, mut children)) = self.containers.pop() {
            children.push(Node::Group {
                clip,
                blend,
                opacity: 1.0,
                children: std::mem::take(&mut self.top),
            });
            self.top = children;
        }
    }

    fn fill(&mut self, brush: Brush<'_>) {
        let paint = self.brush_paint(&brush, self.cur());
        self.emit(
            Some(self.cur() * self.fill_rect_font.clone()),
            &brush,
            paint,
        );
    }

    fn fill_glyph(
        &mut self,
        glyph_id: GlyphId,
        brush_transform: Option<skrifa::color::Transform>,
        brush: Brush<'_>,
    ) {
        if self.err.is_some() {
            return;
        }
        let cur = self.cur();
        let shape = match self.glyph_path(glyph_id).map(|p| cur * p) {
            Ok(p) => Some(p),
            Err(e) => {
                self.err = Some(e);
                return;
            }
        };
        let paint = self.brush_paint(
            &brush,
            brush_transform.map_or(cur, |t| {
                cur * Affine::new([
                    f64::from(t.xx),
                    f64::from(t.yx),
                    f64::from(t.xy),
                    f64::from(t.yy),
                    f64::from(t.dx),
                    f64::from(t.dy),
                ])
            }),
        );
        self.emit(shape, &brush, paint);
    }

    fn push_layer(&mut self, composite_mode: skrifa::color::CompositeMode) {
        if self.err.is_some() {
            return;
        }
        let blend = match composite_to_blend(composite_mode) {
            Ok(b) => b,
            Err(e) => {
                self.err = Some(e);
                return;
            }
        };
        self.containers
            .push((None, blend, std::mem::take(&mut self.top)));
    }

    fn pop_layer_with_mode(&mut self, composite_mode: skrifa::color::CompositeMode) {
        if self.err.is_some() {
            return;
        }
        if let Some((clip, _stored, mut children)) = self.containers.pop() {
            let blend = match composite_to_blend(composite_mode) {
                Ok(b) => b,
                Err(e) => {
                    self.err = Some(e);
                    return;
                }
            };
            children.push(Node::Group {
                clip,
                blend,
                opacity: 1.0,
                children: std::mem::take(&mut self.top),
            });
            self.top = children;
        }
    }
}

/// Wrap a brush-local paint in its paint transform; identity is a no-op.
fn transformed(paint: Paint, tf: Affine) -> Paint {
    if tf == Affine::IDENTITY {
        paint
    } else {
        Paint::Transformed {
            paint: Box::new(paint),
            transform: tf,
        }
    }
}

/// Whether `paint` carries no alpha channel an opacity could scale:
/// image, mesh and shader paints. A foreground brush's COLR `alpha`
/// then needs a group instead of a colour multiply.
fn opacity_needs_group(paint: &Paint) -> bool {
    match paint {
        Paint::Transformed { paint, .. } => opacity_needs_group(paint),
        Paint::Image(_) | Paint::Mesh(_) => true,
        _ => false,
    }
}

impl ColrPainter<'_> {
    /// Emit a fill node — or, for a foreground brush whose `alpha` cannot
    /// fold into the run paint, the fill wrapped in an opacity group.
    fn emit(&mut self, shape: Option<BezPath>, brush: &Brush<'_>, paint: Paint) {
        if let Brush::Solid {
            palette_index: 0xFFFF,
            alpha,
        } = *brush
            && alpha < 1.0
            && opacity_needs_group(&paint)
        {
            self.top.push(Node::Group {
                clip: None,
                blend: BlendMode::Normal,
                opacity: alpha,
                children: vec![Node::Fill {
                    shape,
                    paint: self.foreground.clone(),
                }],
            });
            return;
        }
        self.top.push(Node::Fill { shape, paint });
    }
}

/// Transform a paint's geometry by `t` (for moving from font units into
/// scene space). Image paints compose into `transform`.
fn transform_paint(paint: &mut Paint, t: Affine) {
    match paint {
        Paint::Linear(g) => {
            g.start = t * g.start;
            g.end = t * g.end;
        }
        Paint::Radial(g) => {
            let c = t.as_coeffs();
            let scale = c[1].mul_add(-c[2], c[0] * c[3]).abs().sqrt();
            g.center0 = t * g.center0;
            g.center1 = t * g.center1;
            g.r0 *= scale;
            g.r1 *= scale;
        }
        Paint::Sweep(g) => g.center = t * g.center,
        Paint::Mesh(mesh) => {
            *mesh = cherenkov_scene::MeshGradient::new(
                mesh.columns(),
                mesh.rows(),
                mesh.points().iter().map(|point| t * *point).collect(),
                mesh.colors().to_vec(),
            );
        }
        Paint::Image(i) => i.transform = t * i.transform,
        Paint::Transformed { transform, .. } => *transform = t * *transform,
        Paint::Solid(_) => {}
    }
}

/// Multiply a paint's opacity by `alpha`: the colour's alpha for a solid
/// paint, every stop's alpha for a gradient. `Paint::Image` carries no
/// opacity channel in the scene format, so it is left unchanged.
fn paint_opacity(paint: &mut Paint, alpha: f32) {
    let stops = match paint {
        Paint::Linear(g) => Some(&mut g.stops),
        Paint::Radial(g) => Some(&mut g.stops),
        Paint::Sweep(g) => Some(&mut g.stops),
        Paint::Solid(c) => {
            c.components[3] *= alpha;
            None
        }
        Paint::Transformed { paint, .. } => {
            paint_opacity(paint, alpha);
            None
        }
        Paint::Mesh(mesh) => {
            let colors = mesh
                .colors()
                .iter()
                .copied()
                .map(|mut color| {
                    color.components[3] *= alpha;
                    color
                })
                .collect();
            *mesh = cherenkov_scene::MeshGradient::new(
                mesh.columns(),
                mesh.rows(),
                mesh.points().to_vec(),
                colors,
            );
            None
        }
        Paint::Image(_) => None,
    };
    if let Some(stops) = stops {
        for s in stops {
            s.color.components[3] *= alpha;
        }
    }
}

/// Convert a font-units paint tree into scene [`Item`]s under `place`.
fn node_to_item(node: Node, place: Affine, scene_rect: Rect) -> Item {
    match node {
        Node::Fill { shape, paint } => {
            let mut paint = paint;
            transform_paint(&mut paint, place);
            let shape = shape.map_or_else(
                || Shape::Path {
                    path: scene_rect.to_path(1e-9),
                },
                |p| Shape::Path { path: place * p },
            );
            Item::Draw(Draw::Fill {
                shape,
                rule: FillRule::NonZero,
                paint,
            })
        }
        Node::Group {
            clip,
            blend,
            opacity,
            children,
        } => Item::Layer(Layer {
            transform: Affine::IDENTITY,
            clip: clip.map(|p| Shape::Path { path: place * p }),
            opacity: f64::from(opacity),
            blend,
            backdrop: None,
            filter: None,
            backdrop_effect: None,
            scroll_offset: kurbo::Vec2::ZERO,
            projection: None,
            motion: None,
            live: Vec::new(),
            text: None,
            items: children
                .into_iter()
                .map(|n| node_to_item(n, place, scene_rect))
                .collect(),
        }),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BitmapKind {
    Sbix,
    Cbdt,
}

struct BitmapFont {
    kind: BitmapKind,
    strikes: Box<[(f32, u16)]>,
}

impl BitmapFont {
    fn detect(font: &skrifa::FontRef<'_>) -> Result<Option<Self>, GlyphError> {
        use skrifa::raw::TableProvider as _;
        let (kind, format) = if font.data_for_tag(skrifa::Tag::new(b"sbix")).is_some() {
            font.sbix()
                .map_err(|error| GlyphError::Font(error.to_string()))?;
            (BitmapKind::Sbix, BitmapFormat::Sbix)
        } else if font.data_for_tag(skrifa::Tag::new(b"CBDT")).is_some() {
            font.cblc()
                .map_err(|error| GlyphError::Font(error.to_string()))?;
            font.cbdt()
                .map_err(|error| GlyphError::Font(error.to_string()))?;
            (BitmapKind::Cbdt, BitmapFormat::Cbdt)
        } else {
            return Ok(None);
        };
        let strikes = BitmapStrikes::with_format(font, format)
            .ok_or_else(|| GlyphError::Font("invalid bitmap strike tables".into()))?;
        if strikes.is_empty() {
            return Err(GlyphError::Font("bitmap font has no strikes".into()));
        }
        let mut sizes = Vec::with_capacity(strikes.len());
        for (index, strike) in strikes.iter().enumerate() {
            let index = u16::try_from(index)
                .map_err(|_| GlyphError::Font("too many bitmap strikes".into()))?;
            sizes.push((strike.ppem(), index));
        }
        sizes.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        Ok(Some(Self {
            kind,
            strikes: sizes.into_boxed_slice(),
        }))
    }

    fn select(&self, device_ppem: f32) -> u16 {
        if !device_ppem.is_finite() || device_ppem <= 0.0 {
            return self.largest_strike();
        }
        self.strikes
            .iter()
            .copied()
            .find(|(ppem, _)| *ppem >= device_ppem)
            .map_or_else(|| self.largest_strike(), |(_, index)| index)
    }

    fn largest_strike(&self) -> u16 {
        let largest_ppem = self.strikes.last().expect("bitmap font has strikes").0;
        self.strikes
            .iter()
            .find(|(ppem, _)| ppem.to_bits() == largest_ppem.to_bits())
            .expect("largest bitmap strike exists")
            .1
    }
}

struct DecodedBitmap {
    image: crate::image::Image,
    hash: ResourceHash,
    em: Rect,
}

fn decode_bitmap(
    font: &skrifa::FontRef<'_>,
    bitmap_font: &BitmapFont,
    strike_index: u16,
    glyph_id: u16,
) -> Result<Option<DecodedBitmap>, GlyphError> {
    use skrifa::raw::TableProvider as _;
    let gid = GlyphId::from(glyph_id);
    let format = match bitmap_font.kind {
        BitmapKind::Sbix => BitmapFormat::Sbix,
        BitmapKind::Cbdt => BitmapFormat::Cbdt,
    };
    let strikes = BitmapStrikes::with_format(font, format)
        .ok_or_else(|| GlyphError::Font("invalid bitmap strike tables".into()))?;
    let strike = strikes
        .get(usize::from(strike_index))
        .ok_or_else(|| GlyphError::Font(format!("bitmap strike {strike_index} is missing")))?;
    let glyph = if let Some(glyph) = strike.get(gid) {
        glyph
    } else {
        match bitmap_font.kind {
            BitmapKind::Sbix => {
                let sbix = font
                    .sbix()
                    .map_err(|error| GlyphError::Font(error.to_string()))?;
                let raw_strike = sbix
                    .strikes()
                    .get(usize::from(strike_index))
                    .map_err(|error| GlyphError::Font(error.to_string()))?;
                let raw = match raw_strike.glyph_data(gid) {
                    Ok(Some(raw)) => raw,
                    Ok(None) | Err(skrifa::raw::ReadError::OutOfBounds) => return Ok(None),
                    Err(error) => return Err(GlyphError::Font(error.to_string())),
                };
                if raw.graphic_type() == skrifa::Tag::new(b"dupe") {
                    let target = raw
                        .data()
                        .get(..2)
                        .ok_or_else(|| GlyphError::Font("invalid sbix dupe glyph data".into()))?;
                    let target = u16::from_be_bytes([target[0], target[1]]);
                    let target_id = GlyphId::from(target);
                    let target_raw = match raw_strike.glyph_data(target_id) {
                        Ok(Some(target_raw)) => target_raw,
                        Ok(None) | Err(skrifa::raw::ReadError::OutOfBounds) => {
                            return Err(GlyphError::Font("sbix dupe target is absent".into()));
                        }
                        Err(error) => return Err(GlyphError::Font(error.to_string())),
                    };
                    if target_raw.graphic_type() == skrifa::Tag::new(b"dupe") {
                        return Err(GlyphError::Font("sbix dupe points to another dupe".into()));
                    }
                    if target_raw.graphic_type() != skrifa::Tag::new(b"png ") {
                        return Err(GlyphError::UnsupportedBitmap(format!(
                            "graphic type {:?}",
                            target_raw.graphic_type()
                        )));
                    }
                    strike.get(target_id).ok_or_else(|| {
                        GlyphError::Font("invalid sbix dupe target glyph metrics".into())
                    })?
                } else if raw.graphic_type() == skrifa::Tag::new(b"png ") {
                    return Err(GlyphError::Font("invalid sbix PNG glyph metrics".into()));
                } else {
                    return Err(GlyphError::UnsupportedBitmap(format!(
                        "graphic type {:?}",
                        raw.graphic_type()
                    )));
                }
            }
            BitmapKind::Cbdt => {
                let cblc = font
                    .cblc()
                    .map_err(|error| GlyphError::Font(error.to_string()))?;
                let size = cblc
                    .bitmap_sizes()
                    .get(usize::from(strike_index))
                    .copied()
                    .ok_or_else(|| {
                        GlyphError::Font(format!("bitmap strike {strike_index} is missing"))
                    })?;
                if size.location(cblc.offset_data(), gid).is_err() {
                    return Ok(None);
                }
                return Err(GlyphError::UnsupportedBitmap(
                    "CBDT glyph data format".into(),
                ));
            }
        }
    };
    decode_bitmap_glyph(font, &glyph)
}

fn decode_bitmap_glyph(
    font: &skrifa::FontRef<'_>,
    glyph: &BitmapGlyph<'_>,
) -> Result<Option<DecodedBitmap>, GlyphError> {
    if glyph.width == 0 || glyph.height == 0 {
        return Ok(None);
    }
    let (rgba, premultiplied, raw) = match &glyph.data {
        BitmapData::Png(bytes) => {
            let (width, height, rgba) = crate::image::decode_png_rgba8(bytes)
                .map_err(|error| GlyphError::Font(format!("bitmap PNG: {error}")))?;
            if width != glyph.width || height != glyph.height {
                return Err(GlyphError::Font(
                    "bitmap glyph image disagrees with its metrics".into(),
                ));
            }
            (rgba, false, bytes)
        }
        BitmapData::Bgra(bytes) => {
            let expected = usize::try_from(glyph.width)
                .ok()
                .and_then(|width| {
                    usize::try_from(glyph.height)
                        .ok()
                        .and_then(|height| width.checked_mul(height))
                })
                .and_then(|pixels| pixels.checked_mul(4));
            if expected != Some(bytes.len()) {
                return Err(GlyphError::Font(
                    "bitmap glyph image disagrees with its metrics".into(),
                ));
            }
            (
                bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .flat_map(|pixel| [pixel[2], pixel[1], pixel[0], pixel[3]])
                    .collect(),
                true,
                bytes,
            )
        }
        BitmapData::Mask(_) => {
            return Err(GlyphError::UnsupportedBitmap("mask data".into()));
        }
    };
    let upem = font
        .head()
        .map_err(|error| GlyphError::Font(error.to_string()))?
        .units_per_em();
    if upem == 0 || glyph.ppem_x <= 0.0 || glyph.ppem_y <= 0.0 {
        return Err(GlyphError::Font("invalid bitmap glyph metrics".into()));
    }
    let x0 = f64::from(glyph.bearing_x) / f64::from(upem)
        + f64::from(glyph.inner_bearing_x) / f64::from(glyph.ppem_x);
    let y = f64::from(glyph.bearing_y) / f64::from(upem)
        + f64::from(glyph.inner_bearing_y) / f64::from(glyph.ppem_y);
    let width = f64::from(glyph.width) / f64::from(glyph.ppem_x);
    let height = f64::from(glyph.height) / f64::from(glyph.ppem_y);
    let (y0, y1) = match glyph.placement_origin {
        Origin::TopLeft => (-y, -y + height),
        Origin::BottomLeft => (-y - height, -y),
    };
    let image = bitmap_image(glyph.width, glyph.height, &rgba, premultiplied);
    Ok(Some(DecodedBitmap {
        image,
        hash: ResourceHash::of(raw),
        em: Rect::new(x0, y0, x0 + width, y1),
    }))
}

fn bitmap_image(width: u32, height: u32, rgba: &[u8], premultiplied: bool) -> crate::image::Image {
    let mut pixels = Vec::with_capacity(rgba.len() / 4);
    for px in rgba.as_chunks::<4>().0 {
        let alpha = f64::from(px[3]) / 255.0;
        if alpha == 0.0 {
            pixels.push([0.0; 4]);
            continue;
        }
        let linear = std::array::from_fn(|channel| {
            let encoded = f64::from(px[channel]) / 255.0;
            srgb_decode(if premultiplied {
                (encoded / alpha).min(1.0)
            } else {
                encoded
            })
        });
        let p3 = linear_srgb_to_linear_p3(linear);
        pixels.push([alpha * p3[0], alpha * p3[1], alpha * p3[2], alpha]);
    }
    crate::image::Image {
        width: usize::try_from(width).expect("bitmap width fits usize"),
        height: usize::try_from(height).expect("bitmap height fits usize"),
        pixels,
    }
}

#[expect(
    clippy::many_single_char_names,
    reason = "affine coefficients and eigenvalue formula use standard notation"
)]
fn sigma_max(transform: Affine) -> f64 {
    let [a, b, c, d, ..] = transform.as_coeffs();
    let p = a.mul_add(a, b * b) + c.mul_add(c, d * d);
    let det = a.mul_add(d, -(b * c));
    p.midpoint(p.mul_add(p, -4.0 * det * det).sqrt()).sqrt()
}

/// Expand a [`GlyphRun`] into scene [`Item`]s.
///
/// Each glyph is drawn at `Size::unscaled()` (font units, **unhinted**) under
/// the run's normalized variation coordinates; the resulting items are
/// placed by the glyph's `(x, y)` origin.
///
/// # Errors
/// `GlyphError` on font parse failures, missing outlines or paint-graph
/// errors; `crate::SceneError` on missing font resources.
#[expect(
    clippy::too_many_lines,
    reason = "outline, COLR and transformed bitmap glyph realization share run setup and precedence"
)]
pub fn items_for_glyph_run(
    run: &GlyphRun,
    transform: Affine,
    resources: &mut Resources,
    scene_rect: Rect,
) -> Result<Vec<Item>, GlyphError> {
    let data = resources
        .font(run.font)
        .map_err(|e| GlyphError::Font(e.to_string()))?
        .clone();
    let font = skrifa::FontRef::from_index(&data, run.font_index)
        .map_err(|e| GlyphError::Font(e.to_string()))?;
    if font.data_for_tag(skrifa::Tag::new(b"SVG ")).is_some() {
        return Err(GlyphError::UnsupportedBitmap("SVG color font".into()));
    }
    let bitmap_font = BitmapFont::detect(&font)?;
    if bitmap_font.is_some() && run.stroke.is_some() {
        return Err(GlyphError::UnsupportedBitmap("glyph-stroke".into()));
    }
    let metrics = font.metrics(Size::unscaled(), LocationRef::default());
    let upem = f64::from(metrics.units_per_em);
    if upem <= 0.0 {
        return Err(GlyphError::Font("zero units_per_em".into()));
    }
    let s = f64::from(run.size) / upem;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "strike selection uses the font's f32 ppem metadata"
    )]
    let device_ppem = (f64::from(run.size) * sigma_max(transform)) as f32;

    let coords: Vec<F2Dot14Coord> = run
        .normalized_coords
        .iter()
        .map(|c| F2Dot14Coord::from_f32(c.value))
        .collect();

    let color_glyphs = font.color_glyphs();
    let palette: Vec<skrifa::color::Color> = font
        .color_palettes()
        .get(0)
        .map(|p| p.colors().to_vec())
        .unwrap_or_default();
    let foreground_color = match &run.paint {
        Paint::Solid(c) => *c,
        // Non-solid run paints still act as the foreground brush; inside a
        // gradient stop only a colour is expressible — sRGB black.
        _ => Color {
            space: ColorSpace::Srgb,
            components: [0.0, 0.0, 0.0, 1.0],
        },
    };

    let mut items = Vec::new();
    for g in &run.glyphs {
        let gid_u16 = u16::try_from(g.id).map_err(|_| GlyphError::GlyphId(g.id))?;
        let gid = GlyphId::from(gid_u16);
        let t = g.transform.unwrap_or(Affine::IDENTITY);
        if !t.is_finite() || !t.inverse().is_finite() {
            return Err(GlyphError::Transform);
        }
        let place = Affine::translate((f64::from(g.x), f64::from(g.y)))
            * t
            * Affine::scale_non_uniform(s, -s);

        if let Some(stroke) = &run.stroke {
            let pen = BezPen(outline_path(&font, gid_u16, &coords)?);
            items.push(Item::Draw(Draw::Stroke {
                shape: Shape::Path {
                    path: place * pen.0,
                },
                stroke: stroke.clone(),
                paint: run.paint.clone(),
            }));
        } else if let Some(color_glyph) = color_glyphs.get(gid) {
            // fill_rect_font: canvas rect expressed in font units.
            let inv = place.inverse();
            let corners = [
                inv * Point::new(scene_rect.x0, scene_rect.y0),
                inv * Point::new(scene_rect.x1, scene_rect.y0),
                inv * Point::new(scene_rect.x1, scene_rect.y1),
                inv * Point::new(scene_rect.x0, scene_rect.y1),
            ];
            let mut fill_rect_font = BezPath::new();
            for (i, c) in corners.iter().enumerate() {
                if i == 0 {
                    fill_rect_font.move_to(*c);
                } else {
                    fill_rect_font.line_to(*c);
                }
            }
            fill_rect_font.close_path();

            let mut painter = ColrPainter {
                font: &font,
                coords: &coords,
                palette: palette.clone(),
                foreground: &run.paint,
                foreground_color,
                fill_rect_font,
                tf: vec![Affine::IDENTITY],
                containers: Vec::new(),
                top: Vec::new(),
                err: None,
            };
            color_glyph
                .paint(LocationRef::new(&coords), &mut painter)
                .map_err(|e| GlyphError::Paint(format!("{e}")))?;
            if let Some(e) = painter.err {
                return Err(e);
            }
            let roots = std::mem::take(&mut painter.top);
            items.extend(
                roots
                    .into_iter()
                    .map(|n| node_to_item(n, place, scene_rect)),
            );
        } else if let Some(bitmap_font) = &bitmap_font {
            let strike = if g.transform.is_none() {
                bitmap_font.select(device_ppem)
            } else {
                let bitmap_place = Affine::translate((f64::from(g.x), f64::from(g.y))) * t;
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "strike selection uses the font's f32 ppem metadata"
                )]
                let ppem = (f64::from(run.size) * sigma_max(transform * bitmap_place)) as f32;
                bitmap_font.select(ppem)
            };
            let Some(bitmap) = decode_bitmap(&font, bitmap_font, strike, gid_u16)? else {
                // Bitmap strikes are per-glyph alternates: a glyph with no
                // bitmap in the strike keeps its outline, and a glyph with
                // neither raises `NoOutline` below.
                let pen = BezPen(outline_path(&font, gid_u16, &coords)?);
                if !pen.0.elements().is_empty() {
                    items.push(Item::Draw(Draw::Fill {
                        shape: Shape::Path {
                            path: place * pen.0,
                        },
                        rule: FillRule::NonZero,
                        paint: run.paint.clone(),
                    }));
                }
                continue;
            };
            resources.insert_image(bitmap.hash, bitmap.image);
            if g.transform.is_none() {
                let dst = Rect::new(
                    f64::from(run.size).mul_add(bitmap.em.x0, f64::from(g.x)),
                    f64::from(run.size).mul_add(bitmap.em.y0, f64::from(g.y)),
                    f64::from(run.size).mul_add(bitmap.em.x1, f64::from(g.x)),
                    f64::from(run.size).mul_add(bitmap.em.y1, f64::from(g.y)),
                );
                items.push(Item::Draw(Draw::Image {
                    image: bitmap.hash,
                    encoding: ImageEncoding::default(),
                    dst,
                    sampling: Sampling::Bilinear,
                }));
            } else {
                let dst = Rect::new(
                    f64::from(run.size) * bitmap.em.x0,
                    f64::from(run.size) * bitmap.em.y0,
                    f64::from(run.size) * bitmap.em.x1,
                    f64::from(run.size) * bitmap.em.y1,
                );
                items.push(Item::Layer(Layer {
                    transform: Affine::translate((f64::from(g.x), f64::from(g.y))) * t,
                    clip: None,
                    opacity: 1.0,
                    blend: BlendMode::Normal,
                    backdrop: None,
                    filter: None,
                    backdrop_effect: None,
                    scroll_offset: kurbo::Vec2::ZERO,
                    projection: None,
                    motion: None,
                    items: vec![Item::Draw(Draw::Image {
                        image: bitmap.hash,
                        encoding: ImageEncoding::default(),
                        dst,
                        sampling: Sampling::Bilinear,
                    })],
                    live: Vec::new(),
                    text: None,
                }));
            }
        } else {
            let pen = BezPen(outline_path(&font, gid_u16, &coords)?);
            if pen.0.elements().is_empty() {
                continue;
            }
            items.push(Item::Draw(Draw::Fill {
                shape: Shape::Path {
                    path: place * pen.0,
                },
                rule: FillRule::NonZero,
                paint: run.paint.clone(),
            }));
        }
    }
    Ok(items)
}

/// Obtain one unhinted outline without applying placement or paint.
fn outline_path(
    font: &skrifa::FontRef<'_>,
    glyph: u16,
    coords: &[F2Dot14Coord],
) -> Result<BezPath, GlyphError> {
    let outlines = font.outline_glyphs();
    let outline = outlines
        .get(GlyphId::from(glyph))
        .ok_or(GlyphError::NoOutline(glyph))?;
    let mut pen = BezPen(BezPath::new());
    outline
        .draw(
            DrawSettings::unhinted(Size::unscaled(), LocationRef::new(coords)),
            &mut pen,
        )
        .map_err(|error| GlyphError::Font(error.to_string()))?;
    Ok(pen.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cherenkov_scene::{Color, Glyph, GlyphRun, Scene};
    use std::sync::atomic::{AtomicU64, Ordering};

    const SBIX: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../scenes/fonts/CherenkovSbixTest.ttf"
    ));
    const CBDT: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../scenes/fonts/NotoColorEmojiSubset.ttf"
    ));
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    fn run(font: ResourceHash, glyph: u32) -> GlyphRun {
        GlyphRun {
            font,
            font_index: 0,
            size: 20.0,
            normalized_coords: Vec::new(),
            glyphs: vec![Glyph {
                id: glyph,
                x: 12.0,
                y: 40.0,
                transform: None,
            }],
            stroke: None,
            paint: Paint::Solid(Color {
                space: ColorSpace::Srgb,
                components: [1.0, 0.0, 0.0, 1.0],
            }),
        }
    }

    #[test]
    fn largest_strike_ties_use_the_lowest_table_index() {
        let font = BitmapFont {
            kind: BitmapKind::Sbix,
            strikes: vec![(32.0, 0), (32.0, 2), (96.0, 1), (96.0, 3)].into_boxed_slice(),
        };
        assert_eq!(font.select(97.0), 1);
        assert_eq!(font.select(f32::NAN), 1);
    }

    fn sbix_resources() -> (std::path::PathBuf, Resources, ResourceHash, u32) {
        let dir = std::env::temp_dir().join(format!(
            "cherenkov-oracle-bitmap-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let hash = Scene::store_resource(&dir, SBIX).expect("store font");
        let font = skrifa::FontRef::from_index(SBIX, 0).expect("font");
        let glyph = font.charmap().map('😀').expect("emoji glyph").to_u32();
        (dir.clone(), Resources::new(dir), hash, glyph)
    }

    fn image_draw(items: &[Item]) -> ResourceHash {
        match items {
            [Item::Draw(Draw::Image { image, .. })] => *image,
            [Item::Layer(layer)] => match layer.items.as_slice() {
                [Item::Draw(Draw::Image { image, .. })] => *image,
                other => panic!("expected one transformed bitmap image, got {other:?}"),
            },
            other => panic!("expected one bitmap image, got {other:?}"),
        }
    }

    /// Outline-less bitmap fonts are real (the Android CBDT emoji fonts
    /// carry no `glyf`/`loca`): a glyph with a strike renders through the
    /// bitmap path, and a glyph with neither a bitmap nor an outline is a
    /// hard `NoOutline` error — never silently dropped.
    #[test]
    fn outline_less_fonts_render_by_bitmap_or_hard_error() {
        for (bytes, label) in [(SBIX, "sbix"), (CBDT, "cbdt")] {
            let dir = std::env::temp_dir().join(format!(
                "cherenkov-oracle-{label}-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            let hash = Scene::store_resource(&dir, bytes).expect("store font");
            let mut resources = Resources::new(dir.clone());
            let font = skrifa::FontRef::from_index(bytes, 0).expect("font");
            let glyph = font.charmap().map('😀').expect("emoji glyph").to_u32();
            let items = items_for_glyph_run(
                &run(hash, glyph),
                Affine::IDENTITY,
                &mut resources,
                Rect::new(0.0, 0.0, 320.0, 200.0),
            );
            let items = items.unwrap_or_else(|e| panic!("{label} strike glyph: {e}"));
            let _ = image_draw(&items);
            let missing = items_for_glyph_run(
                &run(hash, u32::from(u16::MAX)),
                Affine::IDENTITY,
                &mut resources,
                Rect::new(0.0, 0.0, 320.0, 200.0),
            );
            assert!(
                matches!(missing, Err(GlyphError::NoOutline(u16::MAX))),
                "{label} glyph with neither bitmap nor outline must error, got {missing:?}"
            );
            std::fs::remove_dir_all(dir).expect("remove temp resources");
        }
    }

    #[test]
    fn transform_scale_changes_the_selected_sbix_strike() {
        let (dir, mut resources, hash, glyph) = sbix_resources();
        let run = run(hash, glyph);
        let rect = Rect::new(0.0, 0.0, 320.0, 200.0);
        let small = items_for_glyph_run(&run, Affine::IDENTITY, &mut resources, rect)
            .expect("identity transform");
        let small_hash = image_draw(&small);
        let mut scaled_run = run.clone();
        scaled_run.glyphs[0].transform = Some(Affine::scale(2.0));
        let large = items_for_glyph_run(&scaled_run, Affine::IDENTITY, &mut resources, rect)
            .expect("per-glyph scale");
        let large_hash = image_draw(&large);
        assert_ne!(small_hash, large_hash);
        let mut size_40_run = run.clone();
        size_40_run.size = 40.0;
        let size_40 = items_for_glyph_run(&size_40_run, Affine::IDENTITY, &mut resources, rect)
            .expect("size-40 run");
        assert_eq!(large_hash, image_draw(&size_40));
        let small_dims = {
            let image = resources
                .image(small_hash, ImageEncoding::default())
                .expect("small image");
            (image.width, image.height)
        };
        let large_dims = {
            let image = resources
                .image(large_hash, ImageEncoding::default())
                .expect("large image");
            (image.width, image.height)
        };
        assert_eq!(small_dims, (40, 38));
        assert_eq!(large_dims, (120, 113));
        std::fs::remove_dir_all(dir).expect("remove temp resources");
    }

    #[test]
    fn non_invertible_bitmap_glyph_transform_is_an_error() {
        let (dir, mut resources, hash, glyph) = sbix_resources();
        let mut run = run(hash, glyph);
        run.glyphs[0].transform = Some(Affine::scale_non_uniform(0.0, 1.0));
        assert!(matches!(
            items_for_glyph_run(
                &run,
                Affine::IDENTITY,
                &mut resources,
                Rect::new(0.0, 0.0, 320.0, 200.0)
            ),
            Err(GlyphError::Transform)
        ));
        std::fs::remove_dir_all(dir).expect("remove temp resources");
    }

    /// A glyph absent from the strike is not silently dropped: with no
    /// outline either, it is a hard `NoOutline` error.
    #[test]
    fn absent_notdef_bitmap_without_outline_is_an_error() {
        let (dir, mut resources, hash, _) = sbix_resources();
        assert!(matches!(
            items_for_glyph_run(&run(hash, 0), Affine::IDENTITY, &mut resources, Rect::ZERO),
            Err(GlyphError::NoOutline(0))
        ));
        std::fs::remove_dir_all(dir).expect("remove temp resources");
    }
}
