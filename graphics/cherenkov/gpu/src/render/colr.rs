//! `COLRv1` glyph expansion: a [`ColorPainter`] port of
//! `oracle/src/glyphs.rs` that records each glyph's paint graph as a
//! font-space [`Picture`] — font units, y-up. The caller replays it under
//! `translate(x, y) * scale_non_uniform(size/upem, -size/upem)`.
//!
//! Brush transforms are exact: gradient geometry stays in brush-local
//! coordinates under a [`TransformedPaint`](cherenkov::TransformedPaint),
//! so skewed and non-uniformly scaled radial/sweep brushes are evaluated
//! in their own space. Font gradients interpolate in sRGB
//! (`Interpolation::SrgbEncoded`) per the `COLRv1` spec's CSS images
//! semantics.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use kurbo::{Affine, BezPath, Point, Rect, Shape as _};
use skrifa::raw::types::{BoundingBox, F2Dot14};
use skrifa::{
    GlyphId, MetadataProvider,
    color::{Brush, ColorPainter, ColorStop},
    instance::LocationRef,
    outline::{DrawSettings, OutlinePen},
};

use cherenkov::{
    BlendMode, BlendSpace, Color, ColorStop as FrontStop, Draw, Extend, Group, Interpolation,
    LinearGradient, Paint, Picture, RadialGradient, Srgb, StaticRecorder, SweepGradient,
    WorkingColor,
};

use cherenkov::RenderError;

use crate::names;
use crate::render::glyph::{FontData, PendingRaster};

/// A canvas-covering rect in font space: `fill` brushes cover whatever clips
/// enclose them, and the enclosing passes bound them to the surface.
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

/// A node of the paint tree the painter builds, in font units.
enum Node {
    /// Fill `shape` (transform-applied font space) with `paint`. `None`
    /// fills the enclosing clip region.
    Fill {
        /// The shape, or the canvas.
        shape: Option<BezPath>,
        /// The paint.
        paint: Paint,
    },
    /// A group: optional clip + blend mode applied to `children`.
    Group {
        /// The clip path.
        clip: Option<BezPath>,
        /// The composite mode.
        blend: BlendMode,
        /// Group opacity: a foreground brush's COLR `alpha` when the run
        /// paint carries no alpha channel.
        opacity: f32,
        /// The children.
        children: Vec<Self>,
    },
}

/// The skrifa composite mode → the front-end blend mode, exactly the set the
/// oracle maps; anything else rejects the glyph.
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

/// Hashes a paint by value — every field's bits, never a
/// `format!("{paint:?}")` allocation on the per-glyph-per-frame path.
fn hash_paint(hasher: &mut impl Hasher, paint: &Paint) {
    fn bits(h: &mut impl Hasher, v: f64) {
        h.write_u64(v.to_bits());
    }
    fn point(h: &mut impl Hasher, p: Point) {
        bits(h, p.x);
        bits(h, p.y);
    }
    fn color(h: &mut impl Hasher, c: WorkingColor) {
        for v in c.components {
            bits(h, f64::from(v));
        }
    }
    fn stops(h: &mut impl Hasher, stops: &[cherenkov::ColorStop]) {
        h.write_usize(stops.len());
        for s in stops {
            bits(h, f64::from(s.offset));
            color(h, s.color);
        }
    }
    std::mem::discriminant(paint).hash(hasher);
    match paint {
        Paint::Transformed(mapped) => {
            for coefficient in mapped.transform.as_coeffs() {
                bits(hasher, coefficient);
            }
            hash_paint(hasher, &mapped.paint);
        }
        Paint::Solid(c) => color(hasher, *c),
        Paint::Linear(g) => {
            point(hasher, g.start);
            point(hasher, g.end);
            stops(hasher, &g.stops);
            g.extend.hash(hasher);
            g.interpolation.hash(hasher);
        }
        Paint::Radial(g) => {
            point(hasher, g.start_center);
            bits(hasher, g.start_radius);
            point(hasher, g.end_center);
            bits(hasher, g.end_radius);
            stops(hasher, &g.stops);
            g.extend.hash(hasher);
            g.interpolation.hash(hasher);
        }
        Paint::Sweep(g) => {
            point(hasher, g.center);
            bits(hasher, g.start_angle);
            bits(hasher, g.end_angle);
            stops(hasher, &g.stops);
            g.extend.hash(hasher);
            g.interpolation.hash(hasher);
        }
        Paint::Mesh(g) => {
            g.interpolation_mode().hash(hasher);
            g.columns().hash(hasher);
            g.rows().hash(hasher);
            for p in g.points() {
                point(hasher, *p);
            }
            for c in g.colors() {
                color(hasher, *c);
            }
        }
        Paint::Image(p) => {
            p.image.hash(hasher);
            for c in p.transform.as_coeffs() {
                bits(hasher, c);
            }
            p.extend_x.hash(hasher);
            p.extend_y.hash(hasher);
            p.sampling.hash(hasher);
        }
        Paint::Shader(s) => {
            s.shader.hash(hasher);
            hasher.write_usize(s.uniforms.len());
            for v in &s.uniforms {
                bits(hasher, f64::from(*v));
            }
        }
    }
}

/// The `COLRv1` painter: keeps a transform stack (font space), a container
/// stack for clips and composite layers, and emits [`Node`]s.
struct ColrPainter<'a> {
    font: &'a skrifa::FontRef<'a>,
    coords: &'a [F2Dot14],
    palette: Vec<skrifa::color::Color>,
    /// The run's own paint — the COLR "foreground" brush
    /// (`palette_index == 0xFFFF`) for solid brushes.
    foreground: &'a Paint,
    /// `foreground`'s colour as a stop colour. For a non-solid run paint —
    /// which a gradient stop cannot express — opaque black.
    foreground_color: WorkingColor,
    /// Canvas rect in font units — what a `fill` with no glyph clip covers.
    fill_rect_font: BezPath,
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
        if index == 0xFFFF || usize::from(index) >= self.palette.len() {
            let mut c = self.foreground_color;
            c.components[3] *= alpha;
            return c;
        }
        let c = self.palette[usize::from(index)];
        Color::<Srgb>::new([
            f32::from(c.red) / 255.0,
            f32::from(c.green) / 255.0,
            f32::from(c.blue) / 255.0,
            f32::from(c.alpha) / 255.0 * alpha,
        ])
        .to_working()
    }

    fn stops(&self, stops: &[ColorStop]) -> Vec<FrontStop> {
        stops
            .iter()
            .map(|s| FrontStop {
                offset: s.offset,
                color: self.palette_color(s.palette_index, s.alpha),
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

    /// Resolve a COLR brush into a front-end [`Paint`]. Gradient
    /// geometry stays in brush-local font units under a paint transform
    /// `tf`, so non-similarity brushes are exact.
    fn brush_paint(&mut self, brush: &Brush<'_>, tf: Affine) -> Paint {
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
            } => Paint::Linear(LinearGradient {
                start: Point::new(f64::from(p0.x), f64::from(p0.y)),
                end: Point::new(f64::from(p1.x), f64::from(p1.y)),
                stops: self.stops(color_stops),
                extend: self.extend(*e),
                interpolation: Interpolation::SrgbEncoded,
            })
            .transformed(tf),
            Brush::RadialGradient {
                c0,
                r0,
                c1,
                r1,
                color_stops,
                extend: e,
            } => Paint::Radial(RadialGradient {
                start_center: Point::new(f64::from(c0.x), f64::from(c0.y)),
                start_radius: f64::from(*r0),
                end_center: Point::new(f64::from(c1.x), f64::from(c1.y)),
                end_radius: f64::from(*r1),
                stops: self.stops(color_stops),
                extend: self.extend(*e),
                interpolation: Interpolation::SrgbEncoded,
            })
            .transformed(tf),
            Brush::SweepGradient {
                c0,
                start_angle,
                end_angle,
                color_stops,
                extend: e,
            } => Paint::Sweep(SweepGradient {
                center: Point::new(f64::from(c0.x), f64::from(c0.y)),
                // skrifa hands degrees, interpreted clockwise in y-up font
                // space; after the y-flip into y-down scene space the same
                // angles read clockwise on screen, which is our convention.
                start_angle: f64::from(*start_angle).to_radians(),
                end_angle: f64::from(*end_angle).to_radians(),
                stops: self.stops(color_stops),
                extend: self.extend(*e),
                interpolation: Interpolation::SrgbEncoded,
            })
            .transformed(tf),
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
        let paint = self.brush_paint(&brush, brush_transform.map_or(cur, |t| cur * to_affine(t)));
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

/// Whether `paint` carries no alpha channel an opacity could scale:
/// image, mesh and shader paints. A foreground brush's COLR `alpha`
/// then needs a group instead of a colour multiply.
fn opacity_needs_group(paint: &Paint) -> bool {
    match paint {
        Paint::Transformed(mapped) => opacity_needs_group(&mapped.paint),
        Paint::Image(_) | Paint::Mesh(_) | Paint::Shader(_) => true,
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

/// Record one node (and its descendants) in font space.
fn record_node(node: &Node, c: &mut StaticRecorder) {
    match node {
        Node::Fill { shape, paint } => {
            let shape = shape.clone().unwrap_or_else(canvas_path);
            c.fill(shape, paint.clone());
        }
        Node::Group {
            clip,
            blend,
            opacity,
            children,
        } => {
            let group = Group {
                opacity: *opacity,
                blend: *blend,
                blend_space: BlendSpace::Linear,
                filter: None,
            };
            let body = |c: &mut StaticRecorder| {
                for n in children {
                    record_node(n, c);
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

/// The font-space [`Picture`] for one COLR glyph, built or fetched from the
/// font's cache. `foreground` is the run's paint — it is baked into the
/// picture, so the cache key hashes it together with the coords.
///
/// `font` is the worker's snapshot of `Renderer::fonts[font_id]`: cache
/// hits read it, a built picture is inserted into it and queued in
/// `pending` so the render thread can commit it to the real font.
pub fn glyph_picture(
    font: &FontData,
    font_id: u64,
    glyph_id: u32,
    coords: &[i16],
    foreground: &Paint,
    pending: &mut Vec<PendingRaster>,
) -> Result<Picture, RenderError> {
    let mut hasher = DefaultHasher::new();
    coords.hash(&mut hasher);
    let coords_hash = hasher.finish();
    let mut hasher = DefaultHasher::new();
    hash_paint(&mut hasher, foreground);
    let paint_hash = hasher.finish();
    let key = (glyph_id, coords_hash, paint_hash);
    let cached = font.colr.borrow().get(&key).cloned();
    if let Some(p) = cached {
        return Ok(p);
    }

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
    let foreground_color = match foreground {
        Paint::Solid(c) => *c,
        _ => WorkingColor::BLACK,
    };
    let mut painter = ColrPainter {
        font: &font_ref,
        coords: &location,
        palette,
        foreground,
        foreground_color,
        fill_rect_font: canvas_path(),
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
    let roots = std::mem::take(&mut painter.top);
    let picture = Picture::record(|c| {
        for n in &roots {
            record_node(n, c);
        }
    });
    font.colr.borrow_mut().insert(key, picture.clone());
    pending.push(PendingRaster::Colr {
        font: font_id,
        key,
        picture: picture.clone(),
    });
    Ok(picture)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cherenkov::{LinearGradient, Sampling};

    fn h(paint: &Paint) -> u64 {
        let mut hasher = DefaultHasher::new();
        hash_paint(&mut hasher, paint);
        hasher.finish()
    }

    /// An unrecognised COLR `Extend` is a corrupt font, not a pad.
    #[test]
    fn an_unknown_extend_is_rejected() {
        use skrifa::color::Extend as SkrifaExtend;
        assert_eq!(extend(SkrifaExtend::Pad), Some(Extend::Pad));
        assert_eq!(extend(SkrifaExtend::Repeat), Some(Extend::Repeat));
        assert_eq!(extend(SkrifaExtend::Reflect), Some(Extend::Reflect));
        assert_eq!(extend(SkrifaExtend::new(7)), None);
    }

    /// The paint hash covers every field that distinguishes paints — the
    /// cache key it feeds must not merge two different foregrounds.
    #[test]
    fn the_paint_hash_covers_every_field() {
        let red = Paint::Solid(WorkingColor::new([1.0, 0.0, 0.0, 1.0]));
        let red_again = Paint::Solid(WorkingColor::new([1.0, 0.0, 0.0, 1.0]));
        let blue = Paint::Solid(WorkingColor::new([0.0, 0.0, 1.0, 1.0]));
        assert_eq!(h(&red), h(&red_again), "equal paints hash equal");
        assert_ne!(h(&red), h(&blue));

        let base = LinearGradient::new((0.0, 0.0), (10.0, 0.0))
            .stop(0.0, WorkingColor::new([1.0, 0.0, 0.0, 1.0]))
            .stop(1.0, WorkingColor::new([0.0, 0.0, 1.0, 1.0]));
        assert_ne!(h(&red), h(&Paint::from(base.clone())), "variants differ");
        assert_ne!(
            h(&Paint::from(base.clone())),
            h(&Paint::from(
                LinearGradient::new((0.0, 0.0), (20.0, 0.0))
                    .stop(0.0, WorkingColor::new([1.0, 0.0, 0.0, 1.0]))
                    .stop(1.0, WorkingColor::new([0.0, 0.0, 1.0, 1.0]))
            )),
            "gradient endpoints"
        );
        assert_ne!(
            h(&Paint::from(base.clone())),
            h(&Paint::from(
                base.clone()
                    .stop(0.5, WorkingColor::new([0.0, 1.0, 0.0, 1.0]))
            )),
            "stop list"
        );
        assert_ne!(
            h(&Paint::from(base.clone().extend(Extend::Repeat))),
            h(&Paint::from(base.clone())),
            "extend"
        );
        assert_ne!(
            h(&Paint::from(
                base.clone().interpolation(Interpolation::SrgbEncoded)
            )),
            h(&Paint::from(base)),
            "interpolation"
        );

        let image = |id, sampling| {
            Paint::Image(cherenkov::ImagePattern {
                image: cherenkov::ImageId::new(id),
                transform: Affine::IDENTITY,
                extend_x: Extend::Pad,
                extend_y: Extend::Pad,
                sampling,
            })
        };
        assert_ne!(
            h(&image(1, Sampling::Linear)),
            h(&image(2, Sampling::Linear))
        );
        assert_ne!(
            h(&image(1, Sampling::Linear)),
            h(&image(1, Sampling::Nearest))
        );
    }
}

#[cfg(test)]
mod mesh_interpolation_key_tests {
    use super::*;
    #[test]
    fn foreground_mesh_mode_is_part_of_color_glyph_identity() {
        use std::hash::Hasher;
        let linear = cherenkov::MeshGradient::new(
            1,
            1,
            vec![
                kurbo::Point::ZERO,
                kurbo::Point::new(1., 0.),
                kurbo::Point::new(0., 1.),
                kurbo::Point::new(1., 1.),
            ],
            vec![cherenkov::WorkingColor::WHITE; 4],
        );
        let smooth = linear
            .clone()
            .interpolation(cherenkov::MeshColorInterpolation::Smoothstep);
        let mut a = std::collections::hash_map::DefaultHasher::new();
        let mut b = std::collections::hash_map::DefaultHasher::new();
        hash_paint(&mut a, &Paint::Mesh(linear));
        hash_paint(&mut b, &Paint::Mesh(smooth));
        assert_ne!(a.finish(), b.finish());
    }
}
