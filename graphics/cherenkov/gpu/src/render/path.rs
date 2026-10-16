//! CPU coverage rasterization of general paths: sparse strips into atlas
//! cells, plus the cache key for replaying them.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use cherenkov::{FillRule, ShapeData};
use kurbo::{Affine, BezPath, PathEl, Point, Rect, Shape as _, Vec2};

use crate::render::glyph::{CellTexels, PathCell, PathEmit};
use crate::render::instance::Instance;
#[cfg(test)]
use crate::render::raster::Raster;
use crate::render::raster::{self, Band, BandRaster};
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
    // Android's emulated TLS expands even a `const { … }` initializer into a
    // non-const init fn, which the lint inspects; native-TLS targets keep it
    // const, so the expectation is Android-only.
    #[cfg_attr(
        target_os = "android",
        expect(
            clippy::missing_const_for_thread_local,
            reason = "the initializer is already const; the lint misfires on std's emulated-TLS expansion (rust-lang/rust-clippy#13422)"
        )
    )]
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
/// `None` when the path misses the surface entirely. `scratch` lends its
/// band rasterizer.
pub fn rasterize(
    segments: &[(f32, f32, f32, f32)],
    bbox: Rect,
    surface: (f64, f64),
    rule: FillRule,
    scratch: &mut EmitScratch,
) -> Option<Coverage> {
    match route() {
        CoverageRoute::Sweep => sweep(segments, bbox, surface, rule, &mut scratch.raster),
    }
}

fn sweep(
    segments: &[(f32, f32, f32, f32)],
    bbox: Rect,
    surface: (f64, f64),
    rule: FillRule,
    scratch: &mut BandRaster,
) -> Option<Coverage> {
    let (x0, y0, w, h, clipped) = grid(bbox, surface)?;
    let (segments, rule) = resolve(segments, rule);
    let mut data = Vec::with_capacity(w * h);
    queue(scratch, &segments, (x0, y0, w, h), rule);
    scratch.finish_choose(&mut |band, _| match band {
        Band::Dense(coverage) => data.extend_from_slice(coverage),
        Band::Sparse(runs, ends) => {
            let mut start = 0;
            for &end in ends {
                let row_start = data.len();
                for &(column, coverage) in &runs[start..end] {
                    data.resize(row_start + column as usize, coverage);
                }
                start = end;
            }
        }
    });
    Some(Coverage {
        x: x0,
        y: y0,
        w,
        h,
        clipped,
        data,
    })
}

/// Reusable path-emission buffers: the band rasterizer and the per-band
/// class runs. The owner keeps one across paths and frames so emission
/// allocates only for growth.
#[derive(Debug, Default)]
pub struct EmitScratch {
    raster: BandRaster,
    /// A band's class runs, which [`partition`] annotates.
    runs: Vec<Run>,
}

impl EmitScratch {
    /// Empty buffers; the first emissions size them.
    pub const fn new() -> Self {
        Self {
            raster: BandRaster::new(),
            runs: Vec::new(),
        }
    }

    /// Heap bytes the buffers hold between emissions.
    pub const fn heap_bytes(&self) -> u64 {
        self.raster.heap_bytes() + (self.runs.capacity() * size_of::<Run>()) as u64
    }
}

/// Rasterizes and emits in one pass: each sealed `STRIP_H`-row coverage
/// band is classified and emitted directly, so the `w × h` coverage grid
/// never materializes. Cell texels append to `arena`; the bool is
/// `Coverage::clipped`.
#[expect(clippy::cast_precision_loss)]
#[expect(clippy::type_complexity)]
pub fn rasterize_emit(
    segments: &[(f32, f32, f32, f32)],
    bbox: Rect,
    surface: (f64, f64),
    rule: FillRule,
    scratch: &mut EmitScratch,
    arena: &mut Vec<u8>,
) -> Option<Result<(PathEmit, Vec<CellTexels>, bool), RenderError>> {
    match route() {
        CoverageRoute::Sweep => {}
    }
    let (x0, y0, w, h, clipped) = grid(bbox, surface)?;
    let (segments, rule) = resolve(segments, rule);
    let EmitScratch { raster, runs } = scratch;
    if (w as f64) <= SMALL_BBOX && (h as f64) <= SMALL_BBOX {
        // One whole-bbox cell needs the texels: collect the (small)
        // coverage and emit it like `emit` does.
        let mut data = Vec::with_capacity(w * h);
        queue(raster, &segments, (x0, y0, w, h), rule);
        raster.finish(&mut |band: &[f32]| data.extend_from_slice(band));
        let coverage = Coverage {
            x: x0,
            y: y0,
            w,
            h,
            clipped,
            data,
        };
        return Some(emit(&coverage, arena).map(|(p, r)| (p, r, clipped)));
    }
    let mut out = PathEmit::default();
    let mut rasters = Vec::new();
    // The grid's area bounds every texel this emission can produce; it
    // is reserved at the first cell, so the arena grows at most once per
    // emitted path and an empty emission never touches it.
    let mut reserve = w * h;
    let mut err = None;
    let mut sink = |band: Band<'_>, sy: usize| {
        if err.is_some() {
            return;
        }
        let r = match band {
            Band::Dense(strip) => emit_strip(
                strip,
                w,
                x0,
                y0,
                sy,
                strip.len() / w,
                runs,
                &mut out,
                &mut rasters,
                arena,
                &mut reserve,
            ),
            Band::Sparse(coverage, ends) => emit_strip_sparse(
                coverage,
                ends,
                w,
                x0,
                y0,
                sy,
                runs,
                &mut out,
                &mut rasters,
                arena,
                &mut reserve,
            ),
        };
        if let Err(e) = r {
            err = Some(e);
        }
    };
    queue(raster, &segments, (x0, y0, w, h), rule);
    raster.finish_choose(&mut sink);
    Some(err.map_or(Ok((out, rasters, clipped)), Err))
}

/// The integer raster grid for `bbox` inflated by 1 px and clipped to
/// the surface rect: origin, size, and whether clipping happened.
#[expect(clippy::cast_possible_truncation)]
#[expect(clippy::cast_sign_loss)]
fn grid(bbox: Rect, surface: (f64, f64)) -> Option<(f64, f64, usize, usize, bool)> {
    let clip = Rect::new(0.0, 0.0, surface.0, surface.1);
    let inflated = bbox.inflate(1.0, 1.0);
    let r = inflated.intersect(clip);
    let clipped = r != inflated;
    let x0 = r.x0.floor();
    let y0 = r.y0.floor();
    let w = (r.x1.ceil() - x0).max(0.0) as usize;
    let h = (r.y1.ceil() - y0).max(0.0) as usize;
    (w > 0 && h > 0).then_some((x0, y0, w, h, clipped))
}

/// The segments `resolve_winding` settles on, borrowed when it leaves the
/// input unchanged.
type Segments<'a> = std::borrow::Cow<'a, [(f32, f32, f32, f32)]>;

/// `resolve_winding`'s output plus the rule it leaves: resolved input
/// rasterizes as `NonZero`.
fn resolve(segments: &[(f32, f32, f32, f32)], rule: FillRule) -> (Segments<'_>, FillRule) {
    let resolved = cherenkov::lowering::resolve_winding(segments, rule);
    let rule = if resolved.is_some() {
        FillRule::NonZero
    } else {
        rule
    };
    (
        resolved.map_or(
            std::borrow::Cow::Borrowed(segments),
            std::borrow::Cow::Owned,
        ),
        rule,
    )
}

/// Queues clipped segments while preserving their input order.
#[expect(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn queue(
    scratch: &mut BandRaster,
    segments: &[(f32, f32, f32, f32)],
    grid: (f64, f64, usize, usize),
    rule: FillRule,
) {
    let (x0, y0, w, h) = grid;
    scratch.reset(w, h, rule);
    scratch.reserve_edges(segments.len());
    let (ox, oy) = (x0 as f32, y0 as f32);
    for &(sx0, sy0, sx1, sy1) in segments {
        clip_x(
            &mut |a, b, c, d| scratch.draw(a, b, c, d),
            sx0 - ox,
            sy0 - oy,
            sx1 - ox,
            sy1 - oy,
            w as f32,
        );
    }
}

/// Clips a segment to `0 <= x <= w` in raster space and draws it into
/// `raster`, keeping the winding deposit the un-clipped edge would have
/// made on the covered columns. Deposits only propagate rightward, so an
/// edge's part at `x < 0` is not dropped: it is drawn as a vertical stub
/// at `x = 0` spanning the same rows, which deposits the full signed area
/// into column 0. Segments at `x > w` are dropped outright — their
/// deposits land in the unread trailing column or beyond. Row clipping is
/// `Raster`'s job: `y` is left untouched.
fn clip_x(draw: &mut impl FnMut(f32, f32, f32, f32), x0: f32, y0: f32, x1: f32, y1: f32, w: f32) {
    if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
        return;
    }
    let dx = x1 - x0;
    let dy = y1 - y0;
    if dx.abs() <= f32::EPSILON {
        if x0 <= w {
            let x = x0.max(0.0);
            draw(x, y0, x, y1);
        }
        return;
    }
    // The rows the segment runs at `x < 0` deposit at column 0: `dx > 0`
    // enters through the left edge at `tz`, `dx < 0` exits through it.
    let tz = -x0 / dx;
    if dx > 0.0 && x0 < 0.0 {
        draw(0.0, y0, 0.0, dy.mul_add(tz.min(1.0), y0));
    } else if dx < 0.0 && x1 < 0.0 {
        draw(0.0, dy.mul_add(tz.max(0.0), y0), 0.0, y1);
    }
    let (enter, exit) = if dx > 0.0 { (0.0, w) } else { (w, 0.0) };
    let t0 = ((enter - x0) / dx).max(0.0);
    let t1 = ((exit - x0) / dx).min(1.0);
    if t0 >= t1 {
        return;
    }
    draw(
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
    let mut rasters = Vec::new();
    let mut reserve = coverage.data.len();
    if coverage.w as f64 <= SMALL_BBOX && coverage.h as f64 <= SMALL_BBOX {
        make_cell(
            &mut |r, columns, arena| {
                arena.extend(
                    coverage.data[r * coverage.w..][columns]
                        .iter()
                        .copied()
                        .map(raster::texel),
                );
            },
            0,
            [
                coverage.x as f32,
                coverage.y as f32,
                (coverage.x + coverage.w as f64) as f32,
                (coverage.y + coverage.h as f64) as f32,
            ],
            coverage.w,
            coverage.h,
            0..0,
            &mut out.cells,
            &mut rasters,
            arena,
            &mut reserve,
        )?;
    } else {
        let mut runs = Vec::new();
        for sy in (0..coverage.h).step_by(STRIP_H) {
            let sh = STRIP_H.min(coverage.h - sy);
            emit_strip(
                &coverage.data[sy * coverage.w..(sy + sh) * coverage.w],
                coverage.w,
                coverage.x,
                coverage.y,
                sy,
                sh,
                &mut runs,
                &mut out,
                &mut rasters,
                arena,
                &mut reserve,
            )?;
        }
    }
    Ok((out, rasters))
}

/// One cell's `w` × `h` coverage rect streamed a row at a time into
/// `R8` texels appended to `arena`, plus its [`PathCell`] record
/// with `rect` in device space. The `middle` columns are full coverage
/// and store no texels: each row stores the columns left of it, then
/// the columns right of it. `reserve` is the emission's pending
/// reservation: its grid area before the first cell, zero after.
#[expect(clippy::too_many_arguments)]
fn make_cell<F: FnMut(usize, std::ops::Range<usize>, &mut Vec<u8>)>(
    append_row: &mut F,
    x: usize,
    rect: [f32; 4],
    w: usize,
    h: usize,
    middle: std::ops::Range<usize>,
    cells: &mut Vec<PathCell>,
    rasters: &mut Vec<CellTexels>,
    arena: &mut Vec<u8>,
    reserve: &mut usize,
) -> Result<(), RenderError> {
    let (Ok(w32), Ok(h32)) = (u32::try_from(w - middle.len()), u32::try_from(h)) else {
        return Err(RenderError::AtlasFull);
    };
    // The emission's first cell reserves its whole grid area, which bounds
    // every later cell of the emission: one growth per emitted path.
    // Reserving each cell instead grows the arena by doubling and makes
    // more reallocations than one per path.
    arena.reserve(std::mem::take(reserve));
    let start = arena.len();
    let (left, right) = (x..x + middle.start, x + middle.end..x + w);
    for row in 0..h {
        if !left.is_empty() {
            append_row(row, left.clone(), arena);
        }
        append_row(row, right.clone(), arena);
    }
    rasters.push((w32, h32, start..arena.len()));
    cells.push(PathCell {
        rect,
        x: 0,
        y: 0,
        // Filled by `Atlas::place_path` on the render thread.
        slot: 0,
        interior: u32::from(u16::try_from(middle.start).map_err(|_| RenderError::AtlasFull)?)
            | (u32::from(u16::try_from(middle.end).map_err(|_| RenderError::AtlasFull)?) << 16),
    });
    Ok(())
}

/// Emits one dense band — `sh * w` coverage values for the `sy`-anchored
/// band of a grid at `cov_x, cov_y` — through [`emit_runs`]. `runs` is
/// the emission's classification scratch, `reserve` its pending
/// reservation (see [`make_cell`]).
#[expect(clippy::too_many_arguments)]
fn emit_strip(
    strip: &[f32],
    w: usize,
    cov_x: f64,
    cov_y: f64,
    sy: usize,
    sh: usize,
    runs: &mut Vec<Run>,
    out: &mut PathEmit,
    rasters: &mut Vec<CellTexels>,
    arena: &mut Vec<u8>,
    reserve: &mut usize,
) -> Result<(), RenderError> {
    dense_runs(strip, w, sh, runs);
    emit_runs(
        runs,
        (cov_x, cov_y, sy, sh),
        &mut |r, columns, arena| {
            arena.extend(strip[r * w..][columns].iter().copied().map(raster::texel));
        },
        out,
        rasters,
        arena,
        reserve,
    )
}

/// `emit_strip`'s sparse twin: the band arrives as per-row `(end_column,
/// coverage)` runs, and only cells materialize texels.
#[expect(clippy::too_many_arguments)]
fn emit_strip_sparse(
    coverage: &[(u32, f32)],
    ends: &[usize],
    w: usize,
    cov_x: f64,
    cov_y: f64,
    sy: usize,
    runs: &mut Vec<Run>,
    out: &mut PathEmit,
    rasters: &mut Vec<CellTexels>,
    arena: &mut Vec<u8>,
    reserve: &mut usize,
) -> Result<(), RenderError> {
    sparse_runs(coverage, ends, w, runs);
    emit_runs(
        runs,
        (cov_x, cov_y, sy, ends.len()),
        &mut |r, columns, arena| {
            let row = &coverage[if r == 0 { 0 } else { ends[r - 1] }..ends[r]];
            let first = row.partition_point(|&(end, _)| end as usize <= columns.start);
            let mut x = columns.start;
            for &(end, coverage) in &row[first..] {
                let end = (end as usize).min(columns.end);
                arena.resize(arena.len() + end - x, raster::texel(coverage));
                x = end;
                if x == columns.end {
                    break;
                }
            }
        },
        out,
        rasters,
        arena,
        reserve,
    )
}

/// Emits a band's class runs as [`partition`] splits them: a span for
/// each full segment, a cell for every other, its rows streamed in by
/// `append_row`. `band` is the grid origin, the band's first row and its
/// height. A cell's widest full run is its middle, which stores no
/// texels (see [`make_cell`]).
#[expect(clippy::cast_possible_truncation)]
#[expect(clippy::cast_precision_loss)]
fn emit_runs<F: FnMut(usize, std::ops::Range<usize>, &mut Vec<u8>)>(
    runs: &mut [Run],
    band: (f64, f64, usize, usize),
    append_row: &mut F,
    out: &mut PathEmit,
    rasters: &mut Vec<CellTexels>,
    arena: &mut Vec<u8>,
    reserve: &mut usize,
) -> Result<(), RenderError> {
    let (cov_x, cov_y, sy, sh) = band;
    partition(runs, sh);
    let top = (cov_y + sy as f64) as f32;
    let bottom = (cov_y + (sy + sh) as f64) as f32;
    let rect = |start: u32, end: u32| {
        [
            (cov_x + f64::from(start)) as f32,
            top,
            (cov_x + f64::from(end)) as f32,
            bottom,
        ]
    };
    let mut k = 0;
    while k < runs.len() {
        let run = runs[k];
        let last = match run.link {
            Link::Skip => {
                k += 1;
                continue;
            }
            Link::Span => {
                out.spans.push(rect(run.start, run.end));
                k += 1;
                continue;
            }
            Link::Head(last) => last as usize,
            Link::Cell(_) => unreachable!("the backward pass marks every cell on its first run"),
        };
        let (start, end) = (run.start, runs[last].end);
        let middle = runs[k..=last]
            .iter()
            .filter(|run| run.class == Class::Full)
            .max_by_key(|run| (run.end - run.start, std::cmp::Reverse(run.start)))
            .map_or(0..0, |run| {
                (run.start - start) as usize..(run.end - start) as usize
            });
        make_cell(
            append_row,
            start as usize,
            rect(start, end),
            (end - start) as usize,
            sh,
            middle,
            &mut out.cells,
            rasters,
            arena,
            reserve,
        )?;
        k = last + 1;
    }
    Ok(())
}

/// What the atlas stores for one band column, judged by comparing its
/// coverage against the quantization's steps ([`raster::TEXEL_NONZERO`],
/// [`raster::TEXEL_FULL`]): coverage that [`raster::texel`] rounds away,
/// such as floating-point residue, never moves the partition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    /// Every texel is 0.
    Empty,
    /// Every texel is `u8::MAX`.
    Full,
    /// Anything else.
    Partial,
}

impl Class {
    /// The class of a column whose rows are all `empty`, or all `full`.
    const fn new(empty: bool, full: bool) -> Self {
        if empty {
            Self::Empty
        } else if full {
            Self::Full
        } else {
            Self::Partial
        }
    }

    /// Folds one coverage value into a column's `(empty, full)` state.
    fn fold((empty, full): (bool, bool), v: f32) -> (bool, bool) {
        (
            empty && v < raster::TEXEL_NONZERO,
            full && v >= raster::TEXEL_FULL,
        )
    }
}

/// What a run became in the band's cheapest emission. The forward pass
/// of [`partition`] sets `Skip`, `Span` or `Cell` on each run as the end
/// of the cheapest prefix; the backward pass then marks the first run of
/// every chosen cell `Head`, and the emission walk only lands on runs
/// that start a choice.
#[derive(Clone, Copy, Debug)]
enum Link {
    /// The run is empty and emits nothing.
    Skip,
    /// The run is full and emits a span.
    Span,
    /// A cell spans from the run at the index through this run.
    Cell(u32),
    /// A cell spans from this run through the run at the index.
    Head(u32),
}

/// A maximal run of equal-class columns in one band. Columns, indices
/// and byte costs are `u32`: a band is at most a surface wide, so they
/// stay far below its range.
#[derive(Clone, Copy, Debug)]
struct Run {
    start: u32,
    end: u32,
    class: Class,
    /// Least byte cost of emitting every run before this one.
    before: u32,
    link: Link,
}

/// Appends columns `start..end` of `class`, extending the last run when
/// it has the same class.
fn push_run(runs: &mut Vec<Run>, start: u32, end: u32, class: Class) {
    match runs.last_mut() {
        Some(run) if run.class == class => run.end = end,
        _ => runs.push(Run {
            start,
            end,
            class,
            before: 0,
            link: Link::Skip,
        }),
    }
}

/// Splits a dense band of `w` columns and `sh` rows into class runs.
fn dense_runs(strip: &[f32], w: usize, sh: usize, runs: &mut Vec<Run>) {
    runs.clear();
    // Runs never outnumber columns: the scratch grows once, to its bound.
    runs.reserve(w);
    let columns = u32::try_from(w).expect("a band is at most a surface wide");
    for x in 0..columns {
        let mut state = (true, true);
        for row in 0..sh {
            state = Class::fold(state, strip[row * w + x as usize]);
        }
        push_run(runs, x, x + 1, Class::new(state.0, state.1));
    }
}

/// Splits a sparse band into class runs. Each row's constant-coverage
/// runs end at column `w`; the band's columns split wherever any row's
/// run ends, and every column between two splits shares one class.
fn sparse_runs(coverage: &[(u32, f32)], ends: &[usize], w: usize, runs: &mut Vec<Run>) {
    runs.clear();
    // Every run ends where some row's coverage run ends, and never
    // outnumbers columns: the scratch grows once, to that bound.
    runs.reserve(w.min(coverage.len()));
    // `ends[r]` bounds row `r`'s runs; the cursors index the flat buffer
    // absolutely and a row's runs always end at column `w`, so no cursor
    // can read past its row.
    let mut cursors = [0usize; STRIP_H];
    let mut base = 0;
    for (cursor, &end) in cursors.iter_mut().zip(ends) {
        *cursor = base;
        base = end;
    }
    let cursors = &mut cursors[..ends.len()];
    let w = u32::try_from(w).expect("a band is at most a surface wide");
    let mut pos = 0;
    while pos < w {
        let mut next = w;
        let mut state = (true, true);
        for &cursor in &*cursors {
            let (end, v) = coverage[cursor];
            next = next.min(end);
            state = Class::fold(state, v);
        }
        push_run(runs, pos, next, Class::new(state.0, state.1));
        pos = next;
        for cursor in cursors.iter_mut() {
            if coverage[*cursor].0 == next {
                *cursor += 1;
            }
        }
    }
}

/// Bytes one instance costs: it is uploaded every frame it draws.
#[expect(
    clippy::cast_possible_truncation,
    reason = "an instance is a few hundred bytes"
)]
const INSTANCE_BYTES: u32 = size_of::<Instance>() as u32;

/// Chooses the band's emission with the least byte cost: every instance
/// costs [`INSTANCE_BYTES`], and every texel a cell stores its byte in
/// the atlas. A cell stores no texels for its widest full run, its
/// middle, so it costs `instance + sh · (width − middle)`. An empty gap
/// is bridged, and a further full run carried as texels, exactly when
/// that is cheaper than the instance a split would add.
///
/// With the middle fixed at a full run `f`, a cell from run `j` through
/// run `k` costs `before_j − sh · start_j − sh · width_f + instance +
/// sh · end_k` for any `j ≤ f ≤ k`, and leaving out a narrower run than
/// the widest only overestimates. The cheapest cell ending at `k` is
/// therefore the lesser of two running minima — over starts `j` alone,
/// and over full runs `f` of the best start up to `f` less `f`'s texels —
/// and one pass solves the band. The backward pass marks each emitted
/// cell on its first run.
#[expect(
    clippy::cast_possible_truncation,
    reason = "runs never outnumber columns"
)]
fn partition(runs: &mut [Run], sh: usize) {
    let sh = i64::from(u32::try_from(sh).expect("a band is at most STRIP_H rows"));
    let instance = i64::from(INSTANCE_BYTES);
    // The cheapest cell start so far as `before_j − sh · start_j`, and
    // the cheapest with a full middle as that less the middle's texels.
    let mut open: Option<(i64, usize)> = None;
    let mut hollow: Option<(i64, usize)> = None;
    for k in 0..runs.len() {
        let run = runs[k];
        let before = i64::from(run.before);
        let (cost, link) = if run.class == Class::Empty {
            (before, Link::Skip)
        } else {
            let here = before - sh * i64::from(run.start);
            if open.is_none_or(|(best, _)| here < best) {
                open = Some((here, k));
            }
            let (best, from) = open.expect("a non-empty run opened a cell start");
            if run.class == Class::Full {
                let middle = best - sh * i64::from(run.end - run.start);
                if hollow.is_none_or(|(best, _)| middle < best) {
                    hollow = Some((middle, from));
                }
            }
            let (best, from) = match hollow {
                Some((middle, from)) if middle < best => (middle, from),
                _ => (best, from),
            };
            let cell = best + instance + sh * i64::from(run.end);
            let span = before + instance;
            if run.class == Class::Full && span <= cell {
                (span, Link::Span)
            } else {
                (cell, Link::Cell(from as u32))
            }
        };
        runs[k].link = link;
        if let Some(next) = runs.get_mut(k + 1) {
            next.before = u32::try_from(cost).expect("a band's byte cost fits u32");
        }
    }
    let mut k = runs.len();
    while k > 0 {
        let last = k - 1;
        k = match runs[last].link {
            Link::Skip | Link::Span => last,
            Link::Cell(from) => {
                runs[from as usize].link = Link::Head(last as u32);
                from as usize
            }
            Link::Head(_) => unreachable!("the forward pass marks no heads"),
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
            &mut EmitScratch::new(),
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
            &mut EmitScratch::new(),
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

    /// A full run between partial runs, however wide, is the middle of
    /// one cell and stores no texels: the cell's texels are its partial
    /// columns alone. A full run with no partial neighbour is a span.
    #[test]
    fn a_wide_full_run_is_a_cell_middle_without_texels() {
        let f = 1000;
        let mut arena = Vec::new();
        let (emitted, rasters) = emit(
            &strip(&[(0, 3), (3 + f, 6 + f)], &[(3, 3 + f)], 2000),
            &mut arena,
        )
        .unwrap();
        assert_eq!(emitted.spans.len(), 0);
        assert_eq!(emitted.cells.len(), 1);
        assert_eq!(emitted.cells[0].rect[2] - emitted.cells[0].rect[0], 1006.0);
        assert_eq!(
            emitted.cells[0].interior,
            3 | (u32::try_from(3 + f).unwrap() << 16)
        );
        assert_eq!((rasters[0].0, rasters[0].1), (6, 4));
        assert!(
            arena[rasters[0].2.clone()]
                .iter()
                .all(|&t| t == raster::texel(0.5)),
            "only the partial columns store texels"
        );
        let (emitted, _) = emit(&strip(&[], &[(10, 20)], 400), &mut arena).unwrap();
        assert_eq!(emitted.spans.len(), 1);
        assert_eq!(emitted.cells.len(), 0);
    }

    /// A cell leaves out only its widest full run. A narrower full run
    /// stays inside as texels while they cost less than the instance a
    /// second cell would add; wider, it becomes that cell's middle.
    #[test]
    fn a_second_full_run_splits_a_cell_only_when_its_texels_cost_more() {
        let limit = size_of::<Instance>() / STRIP_H;
        let wide = 1000;
        let mut arena = Vec::new();
        for (f, cells) in [(limit - 6, 1), (limit + 6, 2)] {
            let second = 6 + wide;
            let (emitted, _) = emit(
                &strip(
                    &[(0, 3), (3 + wide, second), (second + f, second + f + 3)],
                    &[(3, 3 + wide), (second, second + f)],
                    2000,
                ),
                &mut arena,
            )
            .unwrap();
            assert_eq!((emitted.cells.len(), emitted.spans.len()), (cells, 0));
            assert_eq!(emitted.cells[0].middle_width() as usize, wide);
            if cells == 2 {
                assert_eq!(emitted.cells[1].middle_width() as usize, f);
            }
        }
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
        dense_runs(&coverage.data, coverage.w, STRIP_H, &mut runs);
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

#[cfg(test)]
mod emit_eq {
    use super::*;

    #[expect(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    fn dense_emit(
        segments: &[(f32, f32, f32, f32)],
        bbox: Rect,
        surface: (f64, f64),
        rule: FillRule,
    ) -> (PathEmit, Vec<CellTexels>, Vec<u8>, bool) {
        let (x0, y0, w, h, clipped) = grid(bbox, surface).unwrap();
        let (segments, rule) = resolve(segments, rule);
        let (ox, oy) = (x0 as f32, y0 as f32);
        let mut raster = Raster::new(w, h);
        for &(sx0, sy0, sx1, sy1) in segments.iter() {
            clip_x(
                &mut |a, b, c, d| raster.draw_line(a, b, c, d),
                sx0 - ox,
                sy0 - oy,
                sx1 - ox,
                sy1 - oy,
                w as f32,
            );
        }
        let mut arena = Vec::new();
        let (p, r) = emit(
            &Coverage {
                x: x0,
                y: y0,
                w,
                h,
                clipped,
                data: raster.coverage_rule(rule),
            },
            &mut arena,
        )
        .unwrap();
        (p, r, arena, clipped)
    }

    fn check(
        name: &str,
        segments: &[(f32, f32, f32, f32)],
        bbox: Rect,
        surface: (f64, f64),
        rule: FillRule,
    ) {
        let (dp, dr, darena, dc) = dense_emit(segments, bbox, surface, rule);
        let mut scratch = EmitScratch::new();
        let mut barena = Vec::new();
        let (bp, br, bc) = rasterize_emit(segments, bbox, surface, rule, &mut scratch, &mut barena)
            .unwrap()
            .unwrap();
        assert_eq!(dc, bc, "{name}: clipped");
        assert_eq!(dp.spans.len(), bp.spans.len(), "{name}: span count");
        assert_eq!(dp.spans, bp.spans, "{name}: spans");
        assert_eq!(dp.cells.len(), bp.cells.len(), "{name}: cell count");
        for (i, (a, b)) in dp.cells.iter().zip(bp.cells.iter()).enumerate() {
            assert_eq!(a.rect, b.rect, "{name}: cell {i} rect");
            assert_eq!(a.interior, b.interior, "{name}: cell {i} interior");
        }
        assert_eq!(dr.len(), br.len(), "{name}: raster count");
        for (i, (a, b)) in dr.iter().zip(br.iter()).enumerate() {
            assert_eq!((a.0, a.1), (b.0, b.1), "{name}: texel {i} dims");
            assert_eq!(a.2, b.2, "{name}: texel {i} range");
        }
        assert_eq!(darena, barena, "{name}: arena bytes");
    }

    #[test]
    fn sparse_classification_matches_dense_columns() {
        let mut seed = 11u64;
        let mut next = || {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            usize::try_from(seed >> 32).unwrap()
        };
        for _ in 0..2_000 {
            let w = next() % 128 + 1;
            let sh = next() % STRIP_H + 1;
            let mut strip = vec![0.0; w * sh];
            let mut runs = Vec::new();
            let mut ends = Vec::new();
            for row in strip.chunks_mut(w) {
                let mut x = 0;
                while x < w {
                    let end = (x + next() % 16 + 1).min(w);
                    // Residue and near-full coverage quantize to 0 and
                    // `u8::MAX`, so they classify with 0.0 and 1.0.
                    let coverage = [0.0, 1.0, 0.25, 6e-14, 0.999][next() % 5];
                    row[x..end].fill(coverage);
                    runs.push((u32::try_from(end).unwrap(), coverage));
                    x = end;
                }
                ends.push(runs.len());
            }
            let mut dense = Vec::new();
            let mut sparse = Vec::new();
            dense_runs(&strip, w, sh, &mut dense);
            sparse_runs(&runs, &ends, w, &mut sparse);
            let classes = |runs: &[Run]| -> Vec<_> {
                runs.iter()
                    .map(|run| (run.start, run.end, run.class))
                    .collect()
            };
            assert_eq!(classes(&dense), classes(&sparse));
            let (mut dp, mut sp) = (PathEmit::default(), PathEmit::default());
            let (mut dr, mut sr, mut da, mut sa) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            emit_strip(
                &strip, w, 0.0, 0.0, 0, sh, &mut dense, &mut dp, &mut dr, &mut da, &mut 0,
            )
            .unwrap();
            emit_strip_sparse(
                &runs,
                &ends,
                w,
                0.0,
                0.0,
                0,
                &mut sparse,
                &mut sp,
                &mut sr,
                &mut sa,
                &mut 0,
            )
            .unwrap();
            assert_eq!(dp.spans, sp.spans);
            assert_eq!(dp.cells.len(), sp.cells.len());
            for (a, b) in dp.cells.iter().zip(&sp.cells) {
                assert_eq!(a.rect, b.rect);
                assert_eq!(a.interior, b.interior);
            }
            assert_eq!(dr, sr);
            assert_eq!(da, sa);
        }
    }

    #[expect(clippy::suboptimal_flops)]
    #[expect(clippy::cast_precision_loss)]
    #[test]
    fn banded_emit_matches_dense_emit() {
        let surface = (4096.0, 4096.0);
        // Long thin diagonal stroke spanning many bands (the sparse-bbox shape).
        for (name, slope, off) in [
            ("diag-45", 1.0f32, 10.0f32),
            ("diag-shallow", 0.031, 100.0),
            ("diag-steep", 9.7, 5.5),
            ("diag-neg", -0.13, 400.0),
        ] {
            let x0 = 100.0f32;
            let y0 = off;
            let x1 = 3900.0f32;
            let y1 = off + slope * (x1 - x0);
            let t = 1.5f32; // half-stroke-width for a ~3px stroke
            let segs = [
                (x0, y0 - t, x1, y1 - t),
                (x1, y1 - t, x1, y1 + t),
                (x1, y1 + t, x0, y0 + t),
                (x0, y0 + t, x0, y0 - t),
            ];
            let bbox = Rect::new(
                f64::from(x0),
                f64::from(y0.min(y1)) - 4.0,
                f64::from(x1),
                f64::from(y0.max(y1)) + 4.0,
            );
            for rule in [FillRule::NonZero, FillRule::EvenOdd] {
                check(name, &segs, bbox, surface, rule);
            }
        }
        // A zigzag path crossing many band boundaries with both rules.
        let mut zig = Vec::new();
        let (mut x, mut y) = (40.0f32, 40.0f32);
        for i in 0..40 {
            let (nx, ny) = (x + 173.0, y + if i % 2 == 0 { 61.0 } else { -23.0 });
            zig.push((x, y, nx, ny));
            (x, y) = (nx, ny.max(5.0));
        }
        let bbox = Rect::new(0.0, 0.0, 4096.0, 4096.0);
        check("zigzag", &zig, bbox, surface, FillRule::NonZero);
        // Self-intersecting star under EvenOdd.
        let mut star = Vec::new();
        for i in 0..10 {
            let a0 = i as f32 * 0.628_318_55;
            let a1 = (i as f32 + 4.0) * 0.628_318_55;
            star.push((
                500.0 + 480.0 * a0.cos(),
                500.0 + 480.0 * a0.sin(),
                500.0 + 480.0 * a1.cos(),
                500.0 + 480.0 * a1.sin(),
            ));
        }
        check(
            "star-eo",
            &star,
            Rect::new(0.0, 0.0, 1000.0, 1000.0),
            surface,
            FillRule::EvenOdd,
        );
    }
}
