//! CPU coverage rasterization of general paths: sparse strips into atlas
//! cells, plus the cache key for replaying them.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use cherenkov::{FillRule, ShapeData};
use kurbo::{Affine, BezPath, PathEl, Point, Rect, Shape as _, Vec2};

use crate::render::glyph::{CellTexels, PathCell, PathEmit};
use crate::render::instance::Instance;
use crate::render::raster::{self, Raster};
use cherenkov::RenderError;

/// Coverage strip height in device rows.
pub const STRIP_H: usize = 4;

/// A path whose raster bbox fits inside this edge length is emitted as one
/// whole-bbox cell, skipping strips.
const SMALL_BBOX: f64 = 32.0;

/// Flattening tolerance in device pixels.
pub const FLATTEN: f64 = 0.02;

/// The largest singular value of `t`'s linear part — the worst-case factor
/// by which a local distance error grows under the transform. Mirrors
/// `cherenkov_oracle::path::sigma_max`.
#[must_use]
#[expect(
    clippy::many_single_char_names,
    reason = "a/b/c/d are the conventional affine coefficient names"
)]
pub fn sigma_max(t: Affine) -> f64 {
    let [a, b, c, d, _, _] = t.as_coeffs();
    let p = a.mul_add(a, b * b) + c.mul_add(c, d * d);
    let det = a.mul_add(d, -(b * c));
    let disc = p.mul_add(p, (-4.0 * det) * det).sqrt();
    p.midpoint(disc).sqrt()
}

/// A semantic shape as a local `BezPath`: its outline (shared with every
/// other lowering through [`cherenkov::lowering::shape_outline`]), or the
/// open segment of a line.
#[must_use]
pub fn shape_path(shape: &ShapeData, tolerance: f64) -> BezPath {
    match shape {
        ShapeData::Line(l) => l.to_path(tolerance),
        _ => {
            cherenkov::lowering::shape_outline(shape, tolerance)
                .expect("every shape but a line has an outline")
                .0
        }
    }
}

/// Flattens `path` (already in device space) at `tolerance` px into
/// `(x0, y0, x1, y1)` segments and their device bbox. Every subpath is
/// closed: fills close open contours implicitly.
#[expect(clippy::cast_possible_truncation, reason = "device coords are f32")]
pub fn flatten_segments(path: &BezPath, tolerance: f64) -> (Vec<(f32, f32, f32, f32)>, Rect) {
    let mut segments = Vec::new();
    let mut bbox = Rect::new(f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    let mut last = Point::ORIGIN;
    let mut start = Point::ORIGIN;
    let mut line = |p0: Point, p1: Point| {
        if p0 == p1 {
            return;
        }
        bbox = bbox.union_pt(p0).union_pt(p1);
        segments.push((p0.x as f32, p0.y as f32, p1.x as f32, p1.y as f32));
    };
    kurbo::flatten(path.clone(), tolerance, |el| match el {
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
    (segments, bbox)
}

/// Which route turns a path's flattened segments into coverage.
///
/// The CPU sweep is the only route today. water-rs/waterui#2536's GPU
/// coverage route is added by water-rs/waterui#2543: it adds a variant
/// here, grows [`route`] into the predicate of that specification's
/// section 6 over the inputs `route` already takes, and the route
/// differential (`route_diff.rs`) compares the two images under section
/// 4.3 threshold 4.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoverageRoute {
    /// The scanline sweep of this module: `O((n + k) log n)` in `n`
    /// segments and `k` crossings.
    Sweep,
}

// The test-only route override. Decision T2 of the coverage specification:
// large scenes select the GPU route by themselves, and a route is never
// forced through public API — the route-equivalence tests reach this hook
// from inside the crate instead.
#[cfg(test)]
thread_local! {
    static FORCED_ROUTE: std::cell::Cell<Option<CoverageRoute>> =
        const { std::cell::Cell::new(None) };
}

/// Forces `route` for every rasterization on this thread until it is reset
/// with `None`. Test-only, and never reachable from public API.
#[cfg(test)]
pub fn force_route(route: Option<CoverageRoute>) {
    FORCED_ROUTE.with(|forced| forced.set(route));
}

/// The route a rasterization takes. One route exists, so there is nothing
/// to decide and the function takes no inputs: #2543 gives it the
/// route-selection inputs of the coverage specification's section 6 — the
/// flattened segment count, the coverage grid's device bounds, the surface
/// rect and `SharedDevice::has_compute` — along with the variant that
/// needs them.
#[cfg_attr(
    not(test),
    expect(
        clippy::missing_const_for_fn,
        reason = "the test build reads the thread-local override"
    )
)]
fn route() -> CoverageRoute {
    #[cfg(test)]
    if let Some(forced) = FORCED_ROUTE.with(std::cell::Cell::get) {
        return forced;
    }
    CoverageRoute::Sweep
}

/// A coverage grid over an integer device-space rect.
pub struct Coverage {
    /// Device origin of the grid.
    pub x: f64,
    /// Device origin of the grid.
    pub y: f64,
    /// Columns.
    pub w: usize,
    /// Rows.
    pub h: usize,
    /// True when the rasterization bbox was clipped by the surface rect:
    /// the coverage is only valid at the offset it was rasterized under.
    pub clipped: bool,
    /// Row-major coverage in `[0, 1]`.
    pub data: Vec<f32>,
}

/// Rasterizes flattened device-space `segments` spanning `bbox` under
/// `rule`, over `bbox` inflated by 1 px and clipped to the surface rect.
/// `None` when the path misses the surface entirely.
///
/// [`route`] picks which route does it; every route returns the same
/// [`Coverage`] grid, to within section 4.3 threshold 4 of the coverage
/// specification.
pub fn rasterize(
    segments: &[(f32, f32, f32, f32)],
    bbox: Rect,
    surface: (f64, f64),
    rule: FillRule,
) -> Option<Coverage> {
    match route() {
        CoverageRoute::Sweep => sweep(segments, bbox, surface, rule),
    }
}

/// The CPU sweep route: `resolve_winding` turns self-intersections into a
/// non-zero cover and the scanline raster accumulates signed area.
#[expect(clippy::cast_possible_truncation)]
#[expect(clippy::cast_sign_loss)]
#[expect(clippy::cast_precision_loss)]
fn sweep(
    segments: &[(f32, f32, f32, f32)],
    bbox: Rect,
    surface: (f64, f64),
    rule: FillRule,
) -> Option<Coverage> {
    let clip = Rect::new(0.0, 0.0, surface.0, surface.1);
    let inflated = bbox.inflate(1.0, 1.0);
    let r = inflated.intersect(clip);
    let clipped = r != inflated;
    let x0 = r.x0.floor();
    let y0 = r.y0.floor();
    let w = (r.x1.ceil() - x0).max(0.0) as usize;
    let h = (r.y1.ceil() - y0).max(0.0) as usize;
    if w == 0 || h == 0 {
        return None;
    }
    let (ox, oy) = (x0 as f32, y0 as f32);
    let resolved = cherenkov::lowering::resolve_winding(segments, rule);
    let segments = resolved.as_deref().unwrap_or(segments);
    let rule = if resolved.is_some() {
        FillRule::NonZero
    } else {
        rule
    };
    let mut raster = Raster::new(w, h);
    for &(sx0, sy0, sx1, sy1) in segments {
        clip_x(
            &mut raster,
            sx0 - ox,
            sy0 - oy,
            sx1 - ox,
            sy1 - oy,
            w as f32,
        );
    }
    Some(Coverage {
        x: x0,
        y: y0,
        w,
        h,
        clipped,
        data: raster.coverage_rule(rule),
    })
}

/// Clips a segment to `0 <= x <= w` in raster space and draws it into
/// `raster`, keeping the winding deposit the un-clipped edge would have
/// made on the covered columns. Deposits only propagate rightward, so an
/// edge's part at `x < 0` is not dropped: it is drawn as a vertical stub
/// at `x = 0` spanning the same rows, which deposits the full signed area
/// into column 0. Segments at `x > w` are dropped outright — their
/// deposits land in the unread trailing column or beyond. Row clipping is
/// `Raster`'s job: `y` is left untouched.
fn clip_x(raster: &mut Raster, x0: f32, y0: f32, x1: f32, y1: f32, w: f32) {
    if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
        return;
    }
    let dx = x1 - x0;
    let dy = y1 - y0;
    if dx.abs() <= f32::EPSILON {
        if x0 <= w {
            let x = x0.max(0.0);
            raster.draw_line(x, y0, x, y1);
        }
        return;
    }
    // The rows the segment runs at `x < 0` deposit at column 0: `dx > 0`
    // enters through the left edge at `tz`, `dx < 0` exits through it.
    let tz = -x0 / dx;
    if dx > 0.0 && x0 < 0.0 {
        raster.draw_line(0.0, y0, 0.0, dy.mul_add(tz.min(1.0), y0));
    } else if dx < 0.0 && x1 < 0.0 {
        raster.draw_line(0.0, dy.mul_add(tz.max(0.0), y0), 0.0, y1);
    }
    let (enter, exit) = if dx > 0.0 { (0.0, w) } else { (w, 0.0) };
    let t0 = ((enter - x0) / dx).max(0.0);
    let t1 = ((exit - x0) / dx).min(1.0);
    if t0 >= t1 {
        return;
    }
    raster.draw_line(
        dx.mul_add(t0, x0).clamp(0.0, w),
        dy.mul_add(t0, y0),
        dx.mul_add(t1, x0).clamp(0.0, w),
        dy.mul_add(t1, y0),
    );
}

/// Emits `coverage` as spans and atlas cells: one cell when the grid is
/// small, otherwise strips of [`STRIP_H`] rows, each split into full-column
/// spans and cells by [`partition`]'s least byte cost.
///
/// Pure CPU work: the emission and, per cell, its `(w, h, texels)` — the
/// returned cells' `x`/`y` stay zero until the render thread stores them
/// with [`Atlas::store_path`]. Cell order in `cells` matches the per-cell
/// rasters in the second return value index for index. Each raster's
/// texels are appended to `arena`, which the caller owns until the atlas
/// upload has consumed them.
#[expect(clippy::cast_possible_truncation)]
#[expect(clippy::cast_precision_loss)]
pub fn emit(
    coverage: &Coverage,
    arena: &mut Vec<u8>,
) -> Result<(PathEmit, Vec<CellTexels>), RenderError> {
    let mut out = PathEmit::default();
    let mut rasters: Vec<CellTexels> = Vec::new();
    let make_cell = |cells: &mut Vec<PathCell>,
                     rasters: &mut Vec<CellTexels>,
                     arena: &mut Vec<u8>,
                     x: usize,
                     y: usize,
                     w: usize,
                     h: usize,
                     interior: std::ops::Range<usize>|
     -> Result<(), RenderError> {
        let (Ok(w32), Ok(h32)) = (u32::try_from(w), u32::try_from(h)) else {
            return Err(RenderError::AtlasFull);
        };
        // Reserve the coverage's area on an emission's first cell so the
        // arena never regrows mid-emit; empty emissions stay allocation-free.
        if rasters.is_empty() {
            arena.reserve(coverage.data.len());
        }
        let start = arena.len();
        for row in 0..h {
            arena.extend(
                coverage.data[(y + row) * coverage.w + x..][..w]
                    .iter()
                    .copied()
                    .map(raster::texel),
            );
        }
        rasters.push((w32, h32, start..arena.len()));
        cells.push(PathCell {
            rect: [
                (coverage.x + x as f64) as f32,
                (coverage.y + y as f64) as f32,
                (coverage.x + (x + w) as f64) as f32,
                (coverage.y + (y + h) as f64) as f32,
            ],
            x: 0,
            y: 0,
            // Filled by `Atlas::place_path` on the render thread.
            slot: 0,
            interior: u32::from(u16::try_from(interior.start).map_err(|_| RenderError::AtlasFull)?)
                | (u32::from(u16::try_from(interior.end).map_err(|_| RenderError::AtlasFull)?)
                    << 16),
        });
        Ok(())
    };
    if coverage.w as f64 <= SMALL_BBOX && coverage.h as f64 <= SMALL_BBOX {
        make_cell(
            &mut out.cells,
            &mut rasters,
            arena,
            0,
            0,
            coverage.w,
            coverage.h,
            0..0,
        )?;
        return Ok((out, rasters));
    }
    // Runs never outnumber columns: one allocation serves every strip.
    let mut runs = Vec::with_capacity(coverage.w);
    for sy in (0..coverage.h).step_by(STRIP_H) {
        let sh = STRIP_H.min(coverage.h - sy);
        strip_runs(coverage, sy, sh, &mut runs);
        partition(&mut runs, sh);
        let mut k = 0;
        while k < runs.len() {
            let Some(segment) = runs[k].segment else {
                k += 1;
                continue;
            };
            let (start, end) = (runs[k].start, runs[segment.last].end);
            if segment.span {
                out.spans.push([
                    (coverage.x + start as f64) as f32,
                    (coverage.y + sy as f64) as f32,
                    (coverage.x + end as f64) as f32,
                    (coverage.y + (sy + sh) as f64) as f32,
                ]);
            } else {
                // The widest full run inside the cell is its interior.
                let interior = runs[k..=segment.last]
                    .iter()
                    .filter(|run| run.class == Class::Full)
                    .max_by_key(|run| (run.end - run.start, std::cmp::Reverse(run.start)))
                    .map_or(0..0, |run| run.start - start..run.end - start);
                make_cell(
                    &mut out.cells,
                    &mut rasters,
                    arena,
                    start,
                    sy,
                    end - start,
                    sh,
                    interior,
                )?;
            }
            k = segment.last + 1;
        }
    }
    Ok((out, rasters))
}

/// What the atlas stores for one strip column, judged on its texels
/// ([`raster::texel`]): coverage the quantization rounds away, such as
/// floating-point residue, never moves the partition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    /// Every texel is 0.
    Empty,
    /// Every texel is `u8::MAX`.
    Full,
    /// Anything else.
    Partial,
}

/// The last run of an emitted segment that starts at this run, and
/// whether it is a span (one full run) rather than a cell.
#[derive(Clone, Copy, Debug)]
struct Segment {
    last: usize,
    span: bool,
}

/// How the cheapest emission of a strip's runs up to and including one
/// run ends.
#[derive(Clone, Copy, Debug)]
enum Choice {
    /// The run is empty and emits nothing.
    Skip,
    /// The run is full and emits a span.
    Span,
    /// A cell spans from this run back to the run at the index.
    Cell(usize),
}

/// A maximal run of equal-class columns in one coverage strip.
#[derive(Clone, Copy, Debug)]
struct Run {
    start: usize,
    end: usize,
    class: Class,
    /// Least byte cost of emitting every run before this one.
    before: usize,
    choice: Choice,
    /// Set on the first run of each emitted segment.
    segment: Option<Segment>,
}

/// Splits one strip into runs of equal [`Class`], judging each column
/// by comparing against the quantization's steps
/// ([`raster::TEXEL_NONZERO`], [`raster::TEXEL_FULL`]).
fn strip_runs(coverage: &Coverage, sy: usize, sh: usize, runs: &mut Vec<Run>) {
    runs.clear();
    for x in 0..coverage.w {
        let mut empty = true;
        let mut full = true;
        for row in 0..sh {
            let v = coverage.data[(sy + row) * coverage.w + x];
            empty &= v < raster::TEXEL_NONZERO;
            full &= v >= raster::TEXEL_FULL;
        }
        let class = if empty {
            Class::Empty
        } else if full {
            Class::Full
        } else {
            Class::Partial
        };
        match runs.last_mut() {
            Some(run) if run.class == class => run.end = x + 1,
            _ => runs.push(Run {
                start: x,
                end: x + 1,
                class,
                before: 0,
                choice: Choice::Skip,
                segment: None,
            }),
        }
    }
}

/// Chooses the strip's emission with the least byte cost: every
/// instance costs `size_of::<Instance>()`, the bytes it uploads each
/// frame, and every texel a cell carries costs its byte in the atlas. An
/// empty gap is bridged and a full run carried as texels exactly when
/// that is cheaper than the instance a split would add. A cell's cost is
/// linear in its width, so the cheapest cell ending at each run comes
/// from a running minimum, and the whole strip is solved in one pass.
/// Marks each emitted segment on its first run.
fn partition(runs: &mut [Run], sh: usize) {
    let instance = size_of::<Instance>();
    // The cell start j minimising `cost before j − sh · start of j`,
    // compared as `before_j + sh · start_b < before_b + sh · start_j`.
    let mut cell_from: Option<usize> = None;
    for k in 0..runs.len() {
        let before = runs[k].before;
        if runs[k].class != Class::Empty
            && cell_from
                .is_none_or(|b| before + sh * runs[b].start < runs[b].before + sh * runs[k].start)
        {
            cell_from = Some(k);
        }
        let (cost, choice) = match runs[k].class {
            Class::Empty => (before, Choice::Skip),
            class => {
                let from = cell_from.expect("a non-empty run opened a cell start");
                let cell = runs[from].before + instance + sh * (runs[k].end - runs[from].start);
                if class == Class::Full && before + instance <= cell {
                    (before + instance, Choice::Span)
                } else {
                    (cell, Choice::Cell(from))
                }
            }
        };
        runs[k].choice = choice;
        if let Some(next) = runs.get_mut(k + 1) {
            next.before = cost;
        }
    }
    let mut k = runs.len();
    while k > 0 {
        let last = k - 1;
        k = match runs[last].choice {
            Choice::Skip => last,
            Choice::Span => {
                runs[last].segment = Some(Segment { last, span: true });
                last
            }
            Choice::Cell(from) => {
                runs[from].segment = Some(Segment { last, span: false });
                from
            }
        };
    }
}

/// A stable hash of a shape's field bits, for strokes of non-path shapes
/// which have no element list to hash.
fn hash_shape_fields(hasher: &mut impl Hasher, shape: &ShapeData) {
    fn v(hasher: &mut impl Hasher, x: f64) {
        x.to_bits().hash(hasher);
    }
    fn rect(hasher: &mut impl Hasher, r: &Rect) {
        v(hasher, r.x0);
        v(hasher, r.y0);
        v(hasher, r.x1);
        v(hasher, r.y1);
    }
    match shape {
        ShapeData::Rect(r) => {
            0u8.hash(hasher);
            rect(hasher, r);
        }
        ShapeData::RoundedRect(rr) => {
            1u8.hash(hasher);
            rect(hasher, &rr.rect());
            let radii = rr.radii();
            v(hasher, radii.top_left);
            v(hasher, radii.top_right);
            v(hasher, radii.bottom_right);
            v(hasher, radii.bottom_left);
        }
        ShapeData::Continuous(c) => {
            2u8.hash(hasher);
            rect(hasher, &c.rect);
            let radii = c.radii;
            v(hasher, radii.top_left);
            v(hasher, radii.top_right);
            v(hasher, radii.bottom_right);
            v(hasher, radii.bottom_left);
            v(hasher, c.smoothing);
        }
        ShapeData::Circle(c) => {
            3u8.hash(hasher);
            v(hasher, c.center.x);
            v(hasher, c.center.y);
            v(hasher, c.radius);
        }
        ShapeData::Ellipse(e) => {
            4u8.hash(hasher);
            let center = e.center();
            let radii = e.radii();
            v(hasher, center.x);
            v(hasher, center.y);
            v(hasher, radii.x);
            v(hasher, radii.y);
            v(hasher, e.rotation());
        }
        ShapeData::Line(l) => {
            5u8.hash(hasher);
            v(hasher, l.p0.x);
            v(hasher, l.p0.y);
            v(hasher, l.p1.x);
            v(hasher, l.p1.y);
        }
        ShapeData::Path { elements, rule } => {
            6u8.hash(hasher);
            (*rule as u8).hash(hasher);
            hash_elements_into(hasher, elements);
        }
    }
}

/// A stable hash of a stroked draw: the outline's source shape plus every
/// stroke parameter and the local flatten tolerance, tagged so it never
/// collides with a fill of the same geometry.
pub fn hash_stroke(shape: &ShapeData, stroke: &kurbo::Stroke, tolerance: f64) -> u64 {
    let mut hasher = DefaultHasher::new();
    stroke_into(&mut hasher, shape, stroke, tolerance);
    hasher.finish()
}

fn stroke_into(
    hasher: &mut impl Hasher,
    shape: &ShapeData,
    stroke: &kurbo::Stroke,
    tolerance: f64,
) {
    2u64.hash(hasher);
    hash_shape_fields(hasher, shape);
    // The original field sequence, coalesced into native-endian byte writes.
    let mut style = [0; 27];
    style[..8].copy_from_slice(&stroke.width.to_bits().to_ne_bytes());
    style[8] = stroke.join as u8;
    style[9..17].copy_from_slice(&stroke.miter_limit.to_bits().to_ne_bytes());
    style[17] = stroke.start_cap as u8;
    style[18] = stroke.end_cap as u8;
    style[19..].copy_from_slice(&(stroke.dash_pattern.len() as u64).to_ne_bytes());
    hasher.write(&style);
    hasher.write(bytemuck::cast_slice(stroke.dash_pattern.as_slice()));
    let mut tail = [0; 16];
    tail[..8].copy_from_slice(&stroke.dash_offset.to_bits().to_ne_bytes());
    tail[8..].copy_from_slice(&tolerance.to_bits().to_ne_bytes());
    hasher.write(&tail);
}

/// A stable hash of a local path's element list, tagged by draw mode so a
/// fill, an even-odd fill and a stroke of the same outline never collide.
pub fn hash_elements(elements: &[PathEl], tag: u64) -> u64 {
    let mut hasher = DefaultHasher::new();
    tag.hash(&mut hasher);
    hash_elements_into(&mut hasher, elements);
    hasher.finish()
}

fn hash_elements_into(hasher: &mut impl Hasher, elements: &[PathEl]) {
    fn point(bytes: &mut [u8], disc: u8, p: Point) {
        bytes[0] = disc;
        bytes[1..9].copy_from_slice(&p.x.to_bits().to_ne_bytes());
        bytes[9..17].copy_from_slice(&p.y.to_bits().to_ne_bytes());
    }
    for chunk in elements.chunks(8) {
        let mut bytes = [0; 8 * 51];
        let mut used = 0;
        for el in chunk {
            let bytes = &mut bytes[used..used + 51];
            let len = match el {
                PathEl::MoveTo(p) => {
                    point(&mut bytes[..17], 0, *p);
                    17
                }
                PathEl::LineTo(p) => {
                    point(&mut bytes[..17], 1, *p);
                    17
                }
                PathEl::QuadTo(c, p) => {
                    point(&mut bytes[..17], 2, *c);
                    point(&mut bytes[17..34], 2, *p);
                    34
                }
                PathEl::CurveTo(c0, c1, p) => {
                    point(&mut bytes[..17], 3, *c0);
                    point(&mut bytes[17..34], 3, *c1);
                    point(&mut bytes[34..], 3, *p);
                    51
                }
                PathEl::ClosePath => {
                    bytes[0] = 4;
                    1
                }
            };
            used += len;
        }
        hasher.write(&bytes[..used]);
    }
}

/// A path draw's cache key and placement.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    /// Cache key: content hash + matrix + subpixel translation + surface.
    pub key: u64,
    /// The transform to rasterize under: the true 2x2 and the translation's
    /// fractional part.
    pub raster: Affine,
    /// The integer translation the cache's stored rects are relative to.
    pub offset: Vec2,
}

impl Placement {
    /// Offset-specific identity is only needed after the translation-independent
    /// lookup misses, or for coverage clipped by the surface.
    #[expect(clippy::cast_possible_truncation)]
    pub fn key_exact(self) -> u64 {
        let mut bytes = [0; 24];
        bytes[..8].copy_from_slice(&self.key.to_ne_bytes());
        bytes[8..16].copy_from_slice(&(self.offset.x as i64).to_ne_bytes());
        bytes[16..].copy_from_slice(&(self.offset.y as i64).to_ne_bytes());
        let mut hasher = DefaultHasher::new();
        hasher.write(&bytes);
        hasher.finish()
    }
}

/// Builds the [`Placement`] for a draw under `transform` on a
/// `surface`-pixel target. The key holds the 2x2, the translation's
/// fractional part and the surface size, so identical geometry at different
/// integer translations replays the same emission.
#[expect(clippy::cast_possible_truncation)]
#[expect(
    clippy::many_single_char_names,
    reason = "a..f are the conventional affine coefficient names"
)]
pub fn placement(content_hash: u64, transform: Affine, surface: (u32, u32)) -> Placement {
    let [a, b, c, d, e, f] = transform.as_coeffs();
    let ix = e.floor();
    let iy = f.floor();
    let qx = e - ix;
    let qy = f - iy;
    let mut bytes = [0; 40];
    bytes[..8].copy_from_slice(&content_hash.to_ne_bytes());
    for (dst, value) in bytes[8..32]
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip([a, b, c, d, qx, qy])
    {
        *dst = (value as f32).to_bits().to_ne_bytes();
    }
    bytes[32..36].copy_from_slice(&surface.0.to_ne_bytes());
    bytes[36..].copy_from_slice(&surface.1.to_ne_bytes());
    let mut hasher = DefaultHasher::new();
    hasher.write(&bytes);
    let key = hasher.finish();
    Placement {
        key,
        raster: Affine::new([a, b, c, d, ix + qx, iy + qy]),
        offset: Vec2::new(ix, iy),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The translation's fraction is rasterized exactly and hashed as f32
    /// bits, not snapped to the 1/4 px grid.
    #[test]
    fn placement_keeps_the_subpixel_translation() {
        let p = placement(7, Affine::translate((10.2, 3.7)), (64, 64));
        let [_, _, _, _, e, f] = p.raster.as_coeffs();
        assert!((e - 10.2).abs() < 1e-9, "e: {e}");
        assert!((f - 3.7).abs() < 1e-9, "f: {f}");
        assert_eq!(p.offset, Vec2::new(10.0, 3.0));
        // Different fractions hash differently...
        assert_ne!(
            p.key,
            placement(7, Affine::translate((10.0, 3.75)), (64, 64)).key
        );
        // ...while an integer-only shift shares the key.
        assert_eq!(
            p.key,
            placement(7, Affine::translate((25.2, 3.7)), (64, 64)).key
        );
    }

    #[test]
    fn a_left_edge_outside_the_window_still_deposits_coverage() {
        // A rect spanning x = -20..6, y = 1..9 on a 10x10 raster: the
        // left edge sits outside the window, but the winding deposit it
        // would make there must still propagate rightward at column 0.
        let segments = [
            (-20.0, 1.0, 6.0, 1.0),
            (6.0, 1.0, 6.0, 9.0),
            (6.0, 9.0, -20.0, 9.0),
            (-20.0, 9.0, -20.0, 1.0),
        ];
        let coverage = rasterize(
            &segments,
            Rect::new(-20.0, 1.0, 6.0, 9.0),
            (10.0, 10.0),
            FillRule::NonZero,
        )
        .expect("the rect intersects the surface");
        assert!(coverage.clipped, "the bbox hangs off the left edge");
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the fixture coordinates are small non-negative integers"
        )]
        let at = |x: usize, y: usize| {
            coverage.data[(y - coverage.y as usize) * coverage.w + (x - coverage.x as usize)]
        };
        for y in 2..8 {
            for x in 0..6 {
                assert!((at(x, y) - 1.0).abs() < 1e-6, "interior ({x}, {y})");
            }
            assert!(at(6, y).abs() < 1e-6, "past the right edge ({x})", x = 6);
        }
    }

    #[test]
    fn a_slanted_edge_entering_from_the_left_covers_below_it() {
        // Triangle (-20, 2) - (6, 2) - (6, 8) - back to (-20, 2): the
        // hypotenuse crosses the left window edge partway down.
        let segments = [
            (-20.0, 2.0, 6.0, 2.0),
            (6.0, 2.0, 6.0, 8.0),
            (6.0, 8.0, -20.0, 2.0),
        ];
        let coverage = rasterize(
            &segments,
            Rect::new(-20.0, 2.0, 6.0, 8.0),
            (10.0, 10.0),
            FillRule::NonZero,
        )
        .expect("the triangle intersects the surface");
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the fixture coordinates are small non-negative integers"
        )]
        let at = |x: usize, y: usize| {
            coverage.data[(y - coverage.y as usize) * coverage.w + (x - coverage.x as usize)]
        };
        // The hypotenuse crosses x = 0 at y = 8 - 6/26 * 6 = 6.615: rows
        // 2..6 it spans entirely at x < 0 are covered from column 0 on.
        for y in 2..6 {
            assert!(
                (at(0, y) - 1.0).abs() < 1e-6,
                "row under the edge: {}",
                at(0, y)
            );
            assert!((at(3, y) - 1.0).abs() < 1e-6, "interior: {}", at(3, y));
        }
        // Above the crossing (device row 7) the hypotenuse runs inside the
        // window, from x = 1.67 at the row bottom to the vertex at
        // (6, 8): coverage ramps up rightward and column 0 lies outside.
        assert!(at(0, 7).abs() < 1e-6, "left of the hypotenuse");
        assert!(at(4, 7) < at(5, 7), "coverage rises toward the vertex");
        assert!(at(5, 7) > 0.5, "rightmost interior column: {}", at(5, 7));
        // Above the triangle is empty.
        assert!(at(3, 1).abs() < 1e-6, "above the triangle");
    }

    #[test]
    fn even_odd_folds_winding_into_a_triangle_wave() {
        // Two identical 4x4 squares drawn twice: winding 2 in the overlap,
        // so even-odd leaves a hole where non-zero fills.
        let mut raster = Raster::new(8, 8);
        for _ in 0..2 {
            for (x0, y0, x1, y1) in [
                (1.0, 1.0, 5.0, 1.0),
                (5.0, 1.0, 5.0, 5.0),
                (5.0, 5.0, 1.0, 5.0),
                (1.0, 5.0, 1.0, 1.0),
            ] {
                raster.draw_line(x0, y0, x1, y1);
            }
        }
        let eo = raster.coverage_rule(FillRule::EvenOdd);
        let nz = raster.coverage_rule(FillRule::NonZero);
        assert!((nz[3 * 8 + 3] - 1.0).abs() < 1e-6);
        assert!(
            eo[3 * 8 + 3].abs() < 1e-6,
            "even-odd hole: {}",
            eo[3 * 8 + 3]
        );
        // Partial coverage folds the same way: half coverage stays half.
        let mut edge = Raster::new(4, 4);
        edge.draw_line(0.5, 0.0, 0.5, 3.0);
        let eo = edge.coverage_rule(FillRule::EvenOdd);
        assert!((eo[0] - 0.5).abs() < 1e-6, "edge: {}", eo[0]);
    }

    /// A 4-row strip `w` columns wide: `partial` ranges at coverage 0.5,
    /// `full` ranges at 1.0, the rest 0.
    fn strip(partial: &[(usize, usize)], full: &[(usize, usize)], w: usize) -> Coverage {
        let mut data = vec![0.0f32; w * STRIP_H];
        for row in data.chunks_exact_mut(w) {
            for &(a, b) in partial {
                row[a..b].fill(0.5);
            }
            for &(a, b) in full {
                row[a..b].fill(1.0);
            }
        }
        Coverage {
            x: 0.0,
            y: 0.0,
            w,
            h: STRIP_H,
            clipped: false,
            data,
        }
    }

    /// An empty gap between partial runs is carried as zero texels while
    /// its texels cost no more bytes than the instance a second cell
    /// would; one column wider, the cells split.
    #[test]
    fn a_gap_is_bridged_while_its_texels_cost_less_than_an_instance() {
        let g = size_of::<Instance>() / STRIP_H;
        let mut arena = Vec::new();
        let (emitted, rasters) =
            emit(&strip(&[(0, 3), (3 + g, 6 + g)], &[], 400), &mut arena).unwrap();
        assert_eq!(emitted.cells.len(), 1);
        assert_eq!(rasters[0].0 as usize, 6 + g);
        let cell = &arena[rasters[0].2.clone()];
        for row in cell.chunks_exact(6 + g) {
            assert!(
                row[3..3 + g].iter().all(|&t| t == 0),
                "the carried gap is zero coverage"
            );
        }
        let (emitted, _) = emit(&strip(&[(0, 3), (4 + g, 7 + g)], &[], 400), &mut arena).unwrap();
        assert_eq!(emitted.cells.len(), 2);
    }

    /// A full run between partial runs stays inside the cell as its
    /// interior while its texels cost less than the two instances a span
    /// would add; wider, it becomes a span between two cells. A full run
    /// with no partial neighbour is always a span.
    #[test]
    fn a_full_run_splits_a_cell_only_when_its_texels_cost_more() {
        let limit = 2 * size_of::<Instance>() / STRIP_H;
        let mut arena = Vec::new();
        let f = limit - 6;
        let (emitted, _) = emit(
            &strip(&[(0, 3), (3 + f, 6 + f)], &[(3, 3 + f)], 400),
            &mut arena,
        )
        .unwrap();
        assert_eq!(emitted.spans.len(), 0);
        assert_eq!(emitted.cells.len(), 1);
        assert_eq!(
            emitted.cells[0].interior,
            3 | (u32::try_from(3 + f).unwrap() << 16)
        );
        let f = limit + 6;
        let (emitted, _) = emit(
            &strip(&[(0, 3), (3 + f, 6 + f)], &[(3, 3 + f)], 400),
            &mut arena,
        )
        .unwrap();
        assert_eq!(emitted.spans.len(), 1);
        assert_eq!(emitted.cells.len(), 2);
        let (emitted, _) = emit(&strip(&[], &[(10, 20)], 400), &mut arena).unwrap();
        assert_eq!(emitted.spans.len(), 1);
        assert_eq!(emitted.cells.len(), 0);
    }

    /// A column holding only floating-point residue uploads zero texels,
    /// so it classifies empty and the cell starts at the first column the
    /// atlas actually covers.
    #[test]
    fn a_residue_only_column_classifies_empty() {
        let mut coverage = strip(&[(10, 13)], &[], 40);
        for row in coverage.data.as_chunks_mut::<40>().0 {
            row[9] = 6e-14;
        }
        let mut runs = Vec::new();
        strip_runs(&coverage, 0, STRIP_H, &mut runs);
        let classes: Vec<_> = runs
            .iter()
            .map(|run| (run.start, run.end, run.class))
            .collect();
        assert_eq!(
            classes,
            [
                (0, 10, Class::Empty),
                (10, 13, Class::Partial),
                (13, 40, Class::Empty)
            ]
        );
        let mut arena = Vec::new();
        let (emitted, rasters) = emit(&coverage, &mut arena).unwrap();
        assert_eq!(emitted.cells.len(), 1);
        assert_eq!(rasters[0].0, 3, "the cell spans only the uploaded coverage");
    }
}

#[cfg(test)]
mod hash_identity;

#[cfg(test)]
mod route_diff;
