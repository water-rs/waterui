//! The glyph mask cache and mask rasterization.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use cherenkov::GlyphRun;
use cherenkov::kurbo::{Affine, PathEl, Point, Vec2};
use rustc_hash::FxHashMap;
use skrifa::MetadataProvider;
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::TableProvider;
use skrifa::raw::types::F2Dot14;

use crate::render::lower::GlyphReq;
use crate::render::raster::{Accum, Edge};
use cherenkov::{FontData, RenderError};

/// A glyph cache key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    /// The engine font id.
    font: u64,
    /// The glyph index.
    glyph: u32,
    /// `(size * 64).round()` — 1/64th-pixel size granularity.
    size_bits: u32,
    /// Exact subpixel position: `f32` bits of `fx` | `f32` bits of `fy`.
    subpixel: u64,
    /// f32 bits of the device transform's 2x2.
    matrix: [u32; 4],
    /// Hash of the run's variation coordinates.
    coords_hash: u64,
}

/// A rasterized coverage mask: `w * h` cells anchored at `left`/`top`
/// relative to the glyph's integer device origin.
#[derive(Debug)]
pub struct GlyphMask {
    /// Mask's left edge offset from the integer origin.
    pub left: i32,
    /// Mask's top edge offset from the integer origin.
    pub top: i32,
    /// Mask width in cells.
    pub w: u32,
    /// Mask height in cells.
    pub h: u32,
    /// Coverage, `w * h` cells.
    pub cov: Vec<f32>,
}

/// A resolved glyph mask awaiting the band pass.
pub type GlyphSlot = Arc<OnceLock<Arc<GlyphMask>>>;

/// The glyph mask cache: keyed masks plus byte accounting against the
/// CPU budget.
#[derive(Default)]
pub struct GlyphCache {
    map: FxHashMap<GlyphKey, Arc<GlyphMask>>,
    bytes: u64,
    budget: u64,
}

impl GlyphCache {
    /// An empty cache bounded by `budget` bytes.
    pub fn new(budget: u64) -> Self {
        Self {
            map: FxHashMap::default(),
            bytes: 0,
            budget,
        }
    }

    /// A cached mask, if present.
    pub fn get(&self, key: &GlyphKey) -> Option<Arc<GlyphMask>> {
        self.map.get(key).cloned()
    }

    /// Current cached bytes.
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Resize the cache allowance after registered image residency changes.
    pub fn set_budget(&mut self, budget: u64) {
        self.budget = budget;
        if self.bytes > budget {
            self.clear();
        }
    }

    /// Drops every cached mask (`Trim(Critical)` or an over-budget
    /// batch insert, like the GPU atlas's flush-everything policy).
    pub fn clear(&mut self) {
        self.map.clear();
        self.bytes = 0;
    }

    /// Inserts `masks` (one `(key, mask)` pair per missing request).
    /// When the batch would push the cache over budget, everything is
    /// evicted first.
    pub fn insert_batch(&mut self, masks: Vec<(GlyphKey, Arc<GlyphMask>)>) {
        for (key, mask) in masks {
            if self.map.contains_key(&key) {
                continue;
            }
            let bytes = u64::from(mask.w) * u64::from(mask.h) * 4;
            if bytes > self.budget {
                continue;
            }
            if bytes > self.budget.saturating_sub(self.bytes) {
                self.clear();
            }
            self.bytes += bytes;
            self.map.insert(key, mask);
        }
    }
}

/// One glyph's unhinted outline in font units, or `None` when the font
/// has no outline for `id`.
pub fn outline(
    outlines: &skrifa::outline::OutlineGlyphCollection<'_>,
    coords: &[F2Dot14],
    id: u32,
) -> Result<Option<kurbo::BezPath>, RenderError> {
    let Some(glyph) = outlines.get(skrifa::GlyphId::new(id)) else {
        return Ok(None);
    };
    let mut pen = PathPen {
        path: kurbo::BezPath::new(),
    };
    glyph
        .draw(
            DrawSettings::unhinted(
                skrifa::instance::Size::unscaled(),
                skrifa::instance::LocationRef::new(coords),
            ),
            &mut pen,
        )
        .map_err(|e| RenderError::Font(format!("glyph {id}: {e}")))?;
    Ok(Some(pen.path))
}

/// A per-glyph transform must be finite and invertible.
pub fn checked_transform(glyph: &cherenkov::Glyph) -> Result<Affine, RenderError> {
    let t = glyph.transform.unwrap_or(Affine::IDENTITY);
    if !t.is_finite() || !t.inverse().is_finite() {
        return Err(RenderError::Render(
            "glyph transform must be finite and invertible".into(),
        ));
    }
    Ok(t)
}

/// How a glyph is placed: pure translations fold into the position and
/// keep the mask-cache path; anything else is realized as outline
/// coverage.
pub enum GlyphPlacement {
    /// The glyph with a pure translation folded into `x`/`y`.
    Translate(cherenkov::Glyph),
    /// `translate(x, y) * transform`, applied before the font scale.
    Outline(Affine),
}

/// Validates `glyph.transform` and classifies its placement.
#[expect(clippy::float_cmp, reason = "exact identity coefficients")]
#[expect(
    clippy::many_single_char_names,
    reason = "a/b/c/d/e/f are the conventional affine coefficient names"
)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "glyph positions are f32 by design"
)]
pub fn classify(glyph: &cherenkov::Glyph) -> Result<GlyphPlacement, RenderError> {
    if glyph.transform.is_none() {
        return Ok(GlyphPlacement::Translate(*glyph));
    }
    let t = checked_transform(glyph)?;
    let [a, b, c, d, e, f] = t.as_coeffs();
    if a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0 {
        let mut glyph = *glyph;
        glyph.x += e as f32;
        glyph.y += f as f32;
        glyph.transform = None;
        return Ok(GlyphPlacement::Translate(glyph));
    }
    Ok(GlyphPlacement::Outline(
        Affine::translate((f64::from(glyph.x), f64::from(glyph.y))) * t,
    ))
}

/// Unhinted outlines in run coordinates for semantic glyph strokes.
/// Strokes use the font's outline, including on COLR fonts; palette paint
/// graphs apply only to filled glyphs. Missing outlines are explicit errors.
pub fn stroke_outlines(
    font: &FontData,
    run: &cherenkov::GlyphRun,
) -> Result<Vec<kurbo::BezPath>, RenderError> {
    let font_ref = skrifa::FontRef::from_index(&font.data, font.index)
        .map_err(|e| RenderError::Font(e.to_string()))?;
    let upem = font_ref
        .head()
        .map_err(|e| RenderError::Font(e.to_string()))?
        .units_per_em();
    if upem == 0 {
        return Err(RenderError::Font("zero units_per_em".into()));
    }
    let scale = f64::from(run.size) / f64::from(upem);
    let coords: Vec<F2Dot14> = run.coords.iter().map(|c| F2Dot14::from_bits(*c)).collect();
    let outlines = font_ref.outline_glyphs();
    let mut paths = Vec::with_capacity(run.glyphs.len());
    for glyph in run.glyphs.iter() {
        let path = outline(&outlines, &coords, glyph.id)?.ok_or_else(|| {
            RenderError::Font(format!("glyph {} has no stroke outline", glyph.id))
        })?;
        let placement = Affine::translate((f64::from(glyph.x), f64::from(glyph.y)))
            * checked_transform(glyph)?
            * Affine::scale_non_uniform(scale, -scale);
        paths.push(placement * path);
    }
    Ok(paths)
}

/// The cache key for a glyph at a quantized device position — the same
/// key the GPU slice computes.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "size is a small non-negative value"
)]
pub fn glyph_key(run: &GlyphRun, glyph: u32, subpixel: (f32, f32), transform: Affine) -> GlyphKey {
    let mut hasher = DefaultHasher::new();
    run.coords.hash(&mut hasher);
    let [a, b, c, d, ..] = transform.as_coeffs();
    GlyphKey {
        font: run.font.raw(),
        glyph,
        size_bits: (run.size * 64.0).round() as u32,
        subpixel: u64::from(subpixel.0.to_bits()) | (u64::from(subpixel.1.to_bits()) << 32),
        matrix: [
            (a as f32).to_bits(),
            (b as f32).to_bits(),
            (c as f32).to_bits(),
            (d as f32).to_bits(),
        ],
        coords_hash: hasher.finish(),
    }
}

/// An [`OutlinePen`] collecting a glyph outline into a `kurbo::BezPath`.
struct PathPen {
    path: kurbo::BezPath,
}

impl OutlinePen for PathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path.move_to((f64::from(x), f64::from(y)));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.path.line_to((f64::from(x), f64::from(y)));
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.path.quad_to(
            (f64::from(cx0), f64::from(cy0)),
            (f64::from(x), f64::from(y)),
        );
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.path.curve_to(
            (f64::from(cx0), f64::from(cy0)),
            (f64::from(cx1), f64::from(cy1)),
            (f64::from(x), f64::from(y)),
        );
    }

    fn close(&mut self) {
        self.path.close_path();
    }
}

/// An empty mask (missing glyph, empty outline).
const fn empty() -> GlyphMask {
    GlyphMask {
        left: 0,
        top: 0,
        w: 0,
        h: 0,
        cov: Vec::new(),
    }
}

/// Rasterizes one glyph's coverage mask, like the GPU's
/// `glyph::rasterize`: the outline is drawn at `Size::unscaled` in font
/// units, y flipped, scaled by `size / upem`, transformed by the run's
/// 2x2, offset by the quantized subpixel, and flattened to 0.05 device
/// px before exact-area accumulation over the glyph's own bbox.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::many_single_char_names,
    reason = "glyph mask coordinates are small; a/b/c/d/m are affine names"
)]
pub fn rasterize_mask(font: &FontData, req: &GlyphReq) -> Result<GlyphMask, RenderError> {
    let font_ref = skrifa::FontRef::from_index(&font.data, font.index)
        .map_err(|e| RenderError::Font(format!("{e}")))?;
    let upem = font_ref
        .head()
        .map_err(|e| RenderError::Font(format!("head: {e}")))?
        .units_per_em();
    let outlines = font_ref.outline_glyphs();
    let Some(outline) = outlines.get(skrifa::GlyphId::new(req.glyph_id)) else {
        return Ok(empty());
    };
    let location: Vec<F2Dot14> = req.coords.iter().map(|c| F2Dot14::from_bits(*c)).collect();
    let mut pen = PathPen {
        path: kurbo::BezPath::new(),
    };
    let settings = DrawSettings::unhinted(
        skrifa::instance::Size::unscaled(),
        skrifa::instance::LocationRef::new(&location),
    );
    if outline.draw(settings, &mut pen).is_err() || pen.path.is_empty() {
        return Ok(empty());
    }
    // Font units to device pixels: y flips, scale is size per em, then
    // the run's transform's linear part.
    let scale = f64::from(req.size) / f64::from(upem);
    let [a, b, c, d] = req.matrix;
    let m = Affine::new([
        f64::from(a),
        f64::from(b),
        f64::from(c),
        f64::from(d),
        0.0,
        0.0,
    ]) * Affine::scale_non_uniform(scale, -scale);
    let (fx, fy) = req.subpixel;
    let offset = Vec2::new(f64::from(fx), f64::from(fy));
    let mut edges: Vec<Edge> = Vec::new();
    let mut bbox = kurbo::Rect::new(f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    let mut last = Point::ORIGIN;
    let mut start = Point::ORIGIN;
    let mut line = |p0: Point, p1: Point| {
        if p0 == p1 {
            return;
        }
        let a = m * p0 + offset;
        let b = m * p1 + offset;
        bbox = bbox.union_pt(a).union_pt(b);
        edges.push(Edge {
            x0: a.x as f32,
            y0: a.y as f32,
            x1: b.x as f32,
            y1: b.y as f32,
        });
    };
    // Every subpath is closed: an open contour is closed implicitly.
    kurbo::flatten(&pen.path, 0.05 / scale.max(1e-6), |el| match el {
        PathEl::MoveTo(p) => {
            line(last, start);
            start = p;
            last = p;
        }
        PathEl::LineTo(p) => {
            line(last, p);
            last = p;
        }
        PathEl::QuadTo(..) | PathEl::CurveTo(..) => unreachable!("flatten emits lines"),
        PathEl::ClosePath => {
            line(last, start);
            last = start;
        }
    });
    line(last, start);
    if edges.is_empty() || bbox.width() <= 0.0 || bbox.height() <= 0.0 {
        return Ok(empty());
    }
    // Overlapping contours resolve to the union's boundary edges, like
    // the path lowering: `None` keeps the glyph bit-identical.
    let (edges, _) = crate::render::lower::resolve_edges(edges, cherenkov::FillRule::NonZero);
    let left = bbox.x0.floor() as i32 - 1;
    let top = bbox.y0.floor() as i32 - 1;
    let right = bbox.x1.ceil() as i32 + 1;
    let bottom = bbox.y1.ceil() as i32 + 1;
    let (w, h) = ((right - left) as usize, (bottom - top) as usize);
    // Rasterize in mask space.
    let mut acc = Accum::new(w, h);
    let (ox, oy) = (left as f32, top as f32);
    for e in &edges {
        acc.draw_line(e.x0 - ox, e.y0 - oy, e.x1 - ox, e.y1 - oy);
    }
    let mut cov = vec![0.0; w * h];
    for y in 0..h {
        acc.coverage_row(y, cherenkov::FillRule::NonZero, 0, w, |x, c| {
            cov[y * w + x] = c;
        });
    }
    Ok(GlyphMask {
        left,
        top,
        w: w as u32,
        h: h as u32,
        cov,
    })
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    #[test]
    fn duplicate_and_oversized_masks_do_not_inflate_cache_bytes() {
        let run = GlyphRun {
            font: cherenkov::FontId::new(1),
            size: 12.0,
            coords: Vec::new().into(),
            glyphs: Vec::new().into(),
            style: cherenkov::GlyphStyle::Fill,
        };
        let key = glyph_key(&run, 1, (0.0, 0.0), Affine::IDENTITY);
        let mask = Arc::new(GlyphMask {
            left: 0,
            top: 0,
            w: 2,
            h: 2,
            cov: vec![1.0; 4],
        });
        let mut cache = GlyphCache::new(16);
        cache.insert_batch(vec![(key, mask.clone()), (key, mask.clone())]);
        assert_eq!(cache.bytes(), 16);
        cache.set_budget(8);
        assert_eq!(cache.bytes(), 0);
        cache.insert_batch(vec![(key, mask)]);
        assert_eq!(cache.bytes(), 0);
    }
}
