//! `COLRv1` glyph expansion: a [`ColorPainter`] port of
//! `gpu/src/render/colr.rs` that caches each glyph's paint graph as a
//! foreground-independent node tree in font units, then records it into a
//! font-space [`Picture`] (font units, y-up). The caller replays it under
//! `translate(x, y) * scale_non_uniform(size/upem, -size/upem)`.
//!
//! Brush transforms are exact: gradient geometry stays in brush-local
//! coordinates under a [`TransformedPaint`](cherenkov::TransformedPaint),
//! and the CPU's native [`PaintData`](super::paint::PaintData) evaluates
//! the full affine. Font gradients interpolate in sRGB
//! (`Interpolation::SrgbEncoded`) per the `COLRv1` spec's CSS images
//! semantics.

use std::sync::Arc;

use kurbo::{Affine, BezPath, Point, Rect, Shape as _};
use skrifa::raw::types::{BoundingBox, F2Dot14};
use skrifa::{
    GlyphId, MetadataProvider,
    color::{Brush as ColrBrush, ColorPainter, ColorStop},
    instance::LocationRef,
    outline::{DrawSettings, OutlinePen},
};

use cherenkov::{
    BlendMode, BlendSpace, Color, ColorStop as FrontStop, Draw, Extend, Group, Interpolation,
    LinearGradient, Paint, Picture, RadialGradient, Srgb, StaticRecorder, SweepGradient,
    WorkingColor,
};

use cherenkov::RenderError;

use super::font::{ColrKey, Font};
use crate::names;

/// A canvas-covering rect in font space: `fill` brushes cover whatever
/// clips enclose them, and the enclosing passes bound them to the surface.
fn canvas_path() -> BezPath {
    Rect::new(-1.0e6, -1.0e6, 1.0e6, 1.0e6).to_path(1e-9)
}

/// Collects path commands into a [`BezPath`], in font units.
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

/// A stop colour: a resolved palette colour, or the run's foreground
/// colour — which is only known when the picture is recorded.
pub enum StopColor {
    /// A palette colour.
    Color(WorkingColor),
    /// The run's foreground colour with this COLR `alpha` multiplied in.
    Foreground { alpha: f32 },
}

/// One gradient stop with an unresolved colour.
pub struct GradStop {
    /// The stop offset.
    pub offset: f32,
    /// The stop colour.
    pub color: StopColor,
}

/// Gradient geometry, in brush-local font units.
pub enum GradientKind {
    /// p0 → p1 (the COLR p2 rotation hint is unused, as upstream).
    Linear { start: Point, end: Point },
    /// Two-point radial.
    Radial {
        start_center: Point,
        start_radius: f64,
        end_center: Point,
        end_radius: f64,
    },
    /// Conic sweep (radians).
    Sweep {
        center: Point,
        start_angle: f64,
        end_angle: f64,
    },
}

/// An unresolved gradient: stops may reference the foreground colour.
pub struct Gradient {
    /// The geometry.
    pub kind: GradientKind,
    /// The stops.
    pub stops: Vec<GradStop>,
    /// The continuation mode.
    pub extend: Extend,
    /// The brush transform (paint coordinates → font space).
    pub tf: Affine,
}

/// A COLR brush before foreground resolution.
pub enum Brush {
    /// A fully resolved paint.
    Paint(Paint),
    /// The run's own paint at this COLR `alpha`.
    Foreground { alpha: f32 },
    /// The run's foreground colour as a solid paint at this COLR `alpha`
    /// (a `Solid` brush whose palette index is out of range).
    ForegroundColor { alpha: f32 },
    /// A gradient whose stops may reference the foreground colour.
    Gradient(Gradient),
}

/// A node of the paint tree the painter builds, in font units. The tree
/// is foreground-independent so it is shared across run paints.
pub enum Node {
    /// Fill `shape` (transform-applied font space) with `brush`. `None`
    /// fills the enclosing clip region.
    Fill {
        /// The shape, or the canvas.
        shape: Option<BezPath>,
        /// The brush.
        brush: Brush,
    },
    /// A group: optional clip + blend mode applied to `children`.
    Group {
        /// The clip path.
        clip: Option<BezPath>,
        /// The composite mode.
        blend: BlendMode,
        /// The children.
        children: Vec<Self>,
    },
}

/// The skrifa composite mode → the front-end blend mode, exactly the set
/// the GPU port and the oracle map; anything else rejects the glyph.
const fn composite_to_blend(mode: skrifa::color::CompositeMode) -> Result<BlendMode, RenderError> {
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
        _ => return Err(RenderError::Unsupported(names::COLOR_FONT)),
    })
}

/// `Pad`/`Repeat`/`Reflect` are the only `Extend`s; anything else is a
/// corrupt font, not a pad.
const fn extend(e: skrifa::color::Extend) -> Option<Extend> {
    match e {
        skrifa::color::Extend::Pad => Some(Extend::Pad),
        skrifa::color::Extend::Repeat => Some(Extend::Repeat),
        skrifa::color::Extend::Reflect => Some(Extend::Reflect),
        _ => None,
    }
}

/// The `COLRv1` painter: keeps a transform stack (font space), a
/// container stack for clips and composite layers, and emits [`Node`]s.
/// The run's foreground paint plays no part here: foreground references
/// stay symbolic until the picture is recorded.
struct ColrPainter<'a> {
    font: &'a skrifa::FontRef<'a>,
    coords: &'a [F2Dot14],
    palette: Vec<skrifa::color::Color>,
    tf: Vec<Affine>,
    containers: Vec<(Option<BezPath>, BlendMode, Vec<Node>)>,
    top: Vec<Node>,
    /// First failure recorded by a `ColorPainter` callback (the trait's
    /// methods cannot return `Result`); checked after `paint()` returns.
    err: Option<RenderError>,
}

impl ColrPainter<'_> {
    fn cur(&self) -> Affine {
        *self.tf.last().unwrap_or(&Affine::IDENTITY)
    }

    fn palette_color(&self, index: u16, alpha: f32) -> WorkingColor {
        let c = self.palette[usize::from(index)];
        Color::<Srgb>::new([
            f32::from(c.red) / 255.0,
            f32::from(c.green) / 255.0,
            f32::from(c.blue) / 255.0,
            f32::from(c.alpha) / 255.0 * alpha,
        ])
        .to_working()
    }

    fn stops(&self, stops: &[ColorStop]) -> Vec<GradStop> {
        stops
            .iter()
            .map(|s| GradStop {
                offset: s.offset,
                color: if s.palette_index == 0xFFFF
                    || usize::from(s.palette_index) >= self.palette.len()
                {
                    StopColor::Foreground { alpha: s.alpha }
                } else {
                    StopColor::Color(self.palette_color(s.palette_index, s.alpha))
                },
            })
            .collect()
    }

    /// A recognised `Extend` value. An unrecognised one is a corrupt
    /// font: record it (`self.err` aborts the walk) — the returned value
    /// is discarded.
    fn extend(&mut self, e: skrifa::color::Extend) -> Extend {
        extend(e).unwrap_or_else(|| {
            self.err
                .get_or_insert(RenderError::Unsupported(names::COLOR_FONT));
            Extend::Pad
        })
    }

    /// Resolve a COLR brush into a [`Brush`]. Gradient geometry stays in
    /// brush-local font units under the paint transform `tf`.
    fn brush(&mut self, brush: &ColrBrush<'_>, tf: Affine) -> Brush {
        match brush {
            ColrBrush::Solid {
                palette_index,
                alpha,
            } => {
                if *palette_index == 0xFFFF {
                    Brush::Foreground { alpha: *alpha }
                } else if usize::from(*palette_index) >= self.palette.len() {
                    Brush::ForegroundColor { alpha: *alpha }
                } else {
                    Brush::Paint(Paint::Solid(self.palette_color(*palette_index, *alpha)))
                }
            }
            ColrBrush::LinearGradient {
                p0,
                p1,
                color_stops,
                extend: e,
            } => Brush::Gradient(Gradient {
                kind: GradientKind::Linear {
                    start: Point::new(f64::from(p0.x), f64::from(p0.y)),
                    end: Point::new(f64::from(p1.x), f64::from(p1.y)),
                },
                stops: self.stops(color_stops),
                extend: self.extend(*e),
                tf,
            }),
            ColrBrush::RadialGradient {
                c0,
                r0,
                c1,
                r1,
                color_stops,
                extend: e,
            } => Brush::Gradient(Gradient {
                kind: GradientKind::Radial {
                    start_center: Point::new(f64::from(c0.x), f64::from(c0.y)),
                    start_radius: f64::from(*r0),
                    end_center: Point::new(f64::from(c1.x), f64::from(c1.y)),
                    end_radius: f64::from(*r1),
                },
                stops: self.stops(color_stops),
                extend: self.extend(*e),
                tf,
            }),
            ColrBrush::SweepGradient {
                c0,
                start_angle,
                end_angle,
                color_stops,
                extend: e,
            } => Brush::Gradient(Gradient {
                kind: GradientKind::Sweep {
                    center: Point::new(f64::from(c0.x), f64::from(c0.y)),
                    // skrifa hands degrees, interpreted clockwise in y-up
                    // font space; after the y-flip into y-down scene space
                    // the same angles read clockwise on screen, which is
                    // our convention.
                    start_angle: f64::from(*start_angle).to_radians(),
                    end_angle: f64::from(*end_angle).to_radians(),
                },
                stops: self.stops(color_stops),
                extend: self.extend(*e),
                tf,
            }),
        }
    }

    fn glyph_path(&self, glyph_id: GlyphId) -> Result<BezPath, RenderError> {
        let glyph = self.font.outline_glyphs().get(glyph_id).ok_or_else(|| {
            RenderError::Font(format!("glyph {} has no outline", glyph_id.to_u32()))
        })?;
        let mut pen = BezPen(BezPath::new());
        glyph
            .draw(
                DrawSettings::unhinted(
                    skrifa::instance::Size::unscaled(),
                    LocationRef::new(self.coords),
                ),
                &mut pen,
            )
            .map_err(|e| RenderError::Font(e.to_string()))?;
        Ok(pen.0)
    }
}

/// A skrifa COLR transform → a kurbo affine.
fn to_affine(t: skrifa::color::Transform) -> Affine {
    Affine::new([
        f64::from(t.xx),
        f64::from(t.yx),
        f64::from(t.xy),
        f64::from(t.yy),
        f64::from(t.dx),
        f64::from(t.dy),
    ])
}

impl ColorPainter for ColrPainter<'_> {
    fn push_transform(&mut self, transform: skrifa::color::Transform) {
        self.tf.push(self.cur() * to_affine(transform));
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
                children: std::mem::take(&mut self.top),
            });
            self.top = children;
        }
    }

    fn fill(&mut self, brush: ColrBrush<'_>) {
        let brush = self.brush(&brush, self.cur());
        self.top.push(Node::Fill { shape: None, brush });
    }

    fn fill_glyph(
        &mut self,
        glyph_id: GlyphId,
        brush_transform: Option<skrifa::color::Transform>,
        brush: ColrBrush<'_>,
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
        let brush = self.brush(&brush, brush_transform.map_or(cur, |t| cur * to_affine(t)));
        self.top.push(Node::Fill { shape, brush });
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
                children: std::mem::take(&mut self.top),
            });
            self.top = children;
        }
    }
}

/// Multiply a paint's opacity by `alpha`: the colour's alpha for a solid
/// paint, every stop's alpha for a gradient. `Paint::Image`, `Mesh` and
/// `Shader` carry no opacity channel; callers wrap those fills in an
/// opacity group instead, so they are left unchanged here.
fn paint_opacity(paint: &mut Paint, alpha: f32) {
    let stops = match paint {
        Paint::Linear(g) => Some(&mut g.stops),
        Paint::Radial(g) => Some(&mut g.stops),
        Paint::Sweep(g) => Some(&mut g.stops),
        Paint::Solid(c) => {
            c.components[3] *= alpha;
            None
        }
        Paint::Transformed(mapped) => {
            paint_opacity(std::sync::Arc::make_mut(&mut mapped.paint), alpha);
            None
        }
        Paint::Image(_) | Paint::Mesh(_) | Paint::Shader(_) => None,
    };
    if let Some(stops) = stops {
        for s in stops {
            s.color.components[3] *= alpha;
        }
    }
}

/// Whether `paint` carries no alpha channel an opacity could scale:
/// image, mesh and shader paints. A foreground brush's COLR `alpha` then
/// needs a group instead of a colour multiply.
fn opacity_needs_group(paint: &Paint) -> bool {
    match paint {
        Paint::Transformed(mapped) => opacity_needs_group(&mapped.paint),
        Paint::Image(_) | Paint::Mesh(_) | Paint::Shader(_) => true,
        _ => false,
    }
}

/// The record-time foreground context: the run's paint plus the colour a
/// gradient stop can express for it (the run colour for a solid paint,
/// opaque black otherwise).
struct Foreground<'a> {
    /// The run's paint.
    paint: &'a Paint,
    /// `paint`'s colour as a stop colour.
    color: WorkingColor,
}

impl Foreground<'_> {
    /// Resolve a [`StopColor`] against the run paint.
    fn color(&self, color: &StopColor) -> WorkingColor {
        match color {
            StopColor::Color(c) => *c,
            StopColor::Foreground { alpha } => {
                let mut c = self.color;
                c.components[3] *= alpha;
                c
            }
        }
    }

    /// Resolve a [`Brush`] into a [`Paint`].
    fn paint(&self, brush: &Brush) -> Paint {
        match brush {
            Brush::Paint(p) => p.clone(),
            Brush::Foreground { alpha } => {
                let mut paint = self.paint.clone();
                paint_opacity(&mut paint, *alpha);
                paint
            }
            Brush::ForegroundColor { alpha } => {
                let mut color = self.color;
                color.components[3] *= alpha;
                Paint::Solid(color)
            }
            Brush::Gradient(g) => {
                let stops: Vec<FrontStop> = g
                    .stops
                    .iter()
                    .map(|s| FrontStop {
                        offset: s.offset,
                        color: self.color(&s.color),
                    })
                    .collect();
                let paint = match &g.kind {
                    GradientKind::Linear { start, end } => Paint::Linear(LinearGradient {
                        start: *start,
                        end: *end,
                        stops,
                        extend: g.extend,
                        interpolation: Interpolation::SrgbEncoded,
                    }),
                    GradientKind::Radial {
                        start_center,
                        start_radius,
                        end_center,
                        end_radius,
                    } => Paint::Radial(RadialGradient {
                        start_center: *start_center,
                        start_radius: *start_radius,
                        end_center: *end_center,
                        end_radius: *end_radius,
                        stops,
                        extend: g.extend,
                        interpolation: Interpolation::SrgbEncoded,
                    }),
                    GradientKind::Sweep {
                        center,
                        start_angle,
                        end_angle,
                    } => Paint::Sweep(SweepGradient {
                        center: *center,
                        start_angle: *start_angle,
                        end_angle: *end_angle,
                        stops,
                        extend: g.extend,
                        interpolation: Interpolation::SrgbEncoded,
                    }),
                };
                paint.transformed(g.tf)
            }
        }
    }
}

/// Record one node (and its descendants) in font space.
fn record_node(node: &Node, fg: &Foreground<'_>, c: &mut StaticRecorder) {
    match node {
        Node::Fill { shape, brush } => {
            let shape = shape.clone().unwrap_or_else(canvas_path);
            // A foreground brush whose `alpha` cannot fold into the run
            // paint becomes an opacity group instead of a colour multiply.
            if let Brush::Foreground { alpha } = brush
                && *alpha < 1.0
                && opacity_needs_group(fg.paint)
            {
                let group = Group {
                    opacity: *alpha,
                    blend: BlendMode::Normal,
                    blend_space: BlendSpace::Linear,
                    filter: None,
                };
                c.group(group, |c| c.fill(shape, fg.paint.clone()));
                return;
            }
            c.fill(shape, fg.paint(brush));
        }
        Node::Group {
            clip,
            blend,
            children,
        } => {
            let group = Group {
                opacity: 1.0,
                blend: *blend,
                blend_space: BlendSpace::Linear,
                filter: None,
            };
            let body = |c: &mut StaticRecorder| {
                for n in children {
                    record_node(n, fg, c);
                }
            };
            if let Some(clip) = clip {
                c.clip(clip.clone(), |c| c.group(group, body));
            } else {
                c.group(group, body);
            }
        }
    }
}

/// Build or fetch the glyph's node tree, then record it as a font-space
/// [`Picture`] with `foreground` as the run's paint (the COLR `0xFFFF`
/// brush). The tree caches per `(glyph, coords)`; `foreground` is baked
/// into the picture at record time. Called from lowering, which holds
/// the only mutable font access.
pub fn glyph_picture(
    font: &mut Font,
    glyph_id: u32,
    coords: &[i16],
    foreground: &Paint,
) -> Result<Picture, RenderError> {
    let nodes = glyph_nodes(font, glyph_id, coords)?;
    let fg = Foreground {
        paint: foreground,
        color: match foreground {
            Paint::Solid(c) => *c,
            _ => WorkingColor::BLACK,
        },
    };
    Ok(Picture::record(|c| {
        for n in &*nodes {
            record_node(n, &fg, c);
        }
    }))
}

/// The glyph's node tree, built once per `(glyph, coords)` key.
fn glyph_nodes(font: &mut Font, glyph_id: u32, coords: &[i16]) -> Result<Arc<[Node]>, RenderError> {
    let key = ColrKey {
        glyph: glyph_id,
        coords: coords.into(),
    };
    if let Some(nodes) = font.colr.get(&key) {
        return Ok(nodes.clone());
    }
    let nodes: Arc<[Node]> = build_nodes(&font.data, glyph_id, coords)?.into();
    font.colr.insert(key, nodes.clone());
    Ok(nodes)
}

/// Walk the glyph's `COLRv1` paint graph into a node tree.
fn build_nodes(
    font: &cherenkov::FontData,
    glyph_id: u32,
    coords: &[i16],
) -> Result<Vec<Node>, RenderError> {
    let font_ref = skrifa::FontRef::from_index(&font.data, font.index)
        .map_err(|e| RenderError::Font(format!("{e}")))?;
    let location: Vec<F2Dot14> = coords.iter().map(|c| F2Dot14::from_bits(*c)).collect();
    let gid = GlyphId::new(glyph_id);
    let color_glyph = font_ref
        .color_glyphs()
        .get(gid)
        .ok_or(RenderError::Unsupported(names::COLOR_FONT))?;
    let palette: Vec<skrifa::color::Color> = font_ref
        .color_palettes()
        .get(0)
        .map(|p| p.colors().to_vec())
        .unwrap_or_default();
    let mut painter = ColrPainter {
        font: &font_ref,
        coords: &location,
        palette,
        tf: vec![Affine::IDENTITY],
        containers: Vec::new(),
        top: Vec::new(),
        err: None,
    };
    color_glyph
        .paint(LocationRef::new(&location), &mut painter)
        .map_err(|e| RenderError::Font(format!("{e}")))?;
    if let Some(e) = painter.err {
        return Err(e);
    }
    Ok(painter.top)
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrifa::raw::TableProvider as _;

    fn test_font() -> Font {
        let data = std::fs::read("../scenes/fonts/CherenkovColrTest.ttf").expect("test font");
        let font_ref = skrifa::FontRef::new(&data).expect("parse");
        assert!(font_ref.colr().is_ok(), "test font carries COLR");
        Font {
            data: cherenkov::FontData {
                data: data.into(),
                index: 0,
            },
            has_colr: true,
            has_bitmap: false,
            colr: rustc_hash::FxHashMap::default(),
        }
    }

    fn glyph_id(codepoint: char) -> u32 {
        let data = std::fs::read("../scenes/fonts/CherenkovColrTest.ttf").expect("test font");
        skrifa::FontRef::new(&data)
            .expect("parse")
            .charmap()
            .map(codepoint)
            .expect("colour glyph mapped")
            .to_u32()
    }

    /// The cache key is structural: different coords are different
    /// entries, and identical inputs share one node tree.
    #[test]
    fn the_colr_cache_key_is_structural() {
        let mut font = test_font();
        let gid = glyph_id('\u{E300}');
        let a = glyph_nodes(&mut font, gid, &[0]).expect("first");
        let b = glyph_nodes(&mut font, gid, &[1]).expect("second");
        assert!(!Arc::ptr_eq(&a, &b), "coords [0] and [1] are distinct keys");
        let c = glyph_nodes(&mut font, gid, &[0]).expect("third");
        assert!(Arc::ptr_eq(&a, &c), "identical inputs share the tree");
    }

    /// Lowering the same colour glyph twice reuses its node tree, and a
    /// different colour glyph gets a tree of its own.
    #[test]
    fn lowered_colr_glyphs_share_only_identical_graphs() {
        let mut font = test_font();
        let e300 = glyph_id('\u{E300}');
        let e301 = glyph_id('\u{E301}');
        let a = glyph_nodes(&mut font, e300, &[]).expect("first lower");
        let b = glyph_nodes(&mut font, e300, &[]).expect("second lower");
        assert!(
            Arc::ptr_eq(&a, &b),
            "a glyph lowered twice reuses its graph"
        );
        let other = glyph_nodes(&mut font, e301, &[]).expect("other glyph");
        assert!(
            !Arc::ptr_eq(&a, &other),
            "distinct colour glyphs do not collide"
        );
    }
}
