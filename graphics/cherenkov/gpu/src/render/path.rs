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
    /// A dense band's [`Columns`].
    columns: Vec<u8>,
    /// A sparse band's [`Run`]s.
    runs: Vec<Run>,
}

impl EmitScratch {
    /// Empty buffers; the first emissions size them.
    pub const fn new() -> Self {
        Self {
            raster: BandRaster::new(),
            columns: Vec::new(),
            runs: Vec::new(),
        }
    }

    /// Heap bytes the buffers hold between emissions.
    pub const fn heap_bytes(&self) -> u64 {
        self.raster.heap_bytes()
            + (self.columns.capacity() + self.runs.capacity() * size_of::<Run>()) as u64
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
    let EmitScratch {
        raster,
        columns,
        runs,
    } = scratch;
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
                columns,
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
        let mut columns = Vec::new();
        for sy in (0..coverage.h).step_by(STRIP_H) {
            let sh = STRIP_H.min(coverage.h - sy);
            emit_strip(
                &coverage.data[sy * coverage.w..(sy + sh) * coverage.w],
                coverage.w,
                coverage.x,
                coverage.y,
                sy,
                sh,
                &mut columns,
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
/// with `rect` in device space and `interior` its full columns from `x`.
/// `reserve` is the emission's pending reservation: its grid area before
/// the first cell, zero after.
#[expect(clippy::too_many_arguments)]
fn make_cell<F: FnMut(usize, std::ops::Range<usize>, &mut Vec<u8>)>(
    append_row: &mut F,
    x: usize,
    rect: [f32; 4],
    w: usize,
    h: usize,
    interior: std::ops::Range<usize>,
    cells: &mut Vec<PathCell>,
    rasters: &mut Vec<CellTexels>,
    arena: &mut Vec<u8>,
    reserve: &mut usize,
) -> Result<(), RenderError> {
    let (Ok(w32), Ok(h32)) = (u32::try_from(w), u32::try_from(h)) else {
        return Err(RenderError::AtlasFull);
    };
    // The emission's first cell reserves its whole grid area, which bounds
    // every later cell of the emission: one growth per emitted path.
    // Reserving each cell instead grows the arena by doubling and makes
    // more reallocations than one per path.
    arena.reserve(std::mem::take(reserve));
    let start = arena.len();
    for row in 0..h {
        append_row(row, x..x + w, arena);
    }
    rasters.push((w32, h32, start..arena.len()));
    cells.push(PathCell {
        rect,
        x: 0,
        y: 0,
        // Filled by `Atlas::place_path` on the render thread.
        slot: 0,
        interior: u32::from(u16::try_from(interior.start).map_err(|_| RenderError::AtlasFull)?)
            | (u32::from(u16::try_from(interior.end).map_err(|_| RenderError::AtlasFull)?) << 16),
    });
    Ok(())
}

/// Emits one dense band — `sh * w` coverage values for the `sy`-anchored
/// band of a grid at `cov_x, cov_y` — through [`emit_runs`]. `columns`
/// is the emission's classification scratch, `reserve` its pending
/// reservation (see [`make_cell`]).
#[expect(clippy::too_many_arguments)]
fn emit_strip(
    strip: &[f32],
    w: usize,
    cov_x: f64,
    cov_y: f64,
    sy: usize,
    sh: usize,
    columns: &mut Vec<u8>,
    out: &mut PathEmit,
    rasters: &mut Vec<CellTexels>,
    arena: &mut Vec<u8>,
    reserve: &mut usize,
) -> Result<(), RenderError> {
    dense_columns(strip, w, sh, columns);
    emit_runs(
        &mut Columns(columns),
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
        runs.as_mut_slice(),
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
/// height. A cell's widest full run, the leftmost of equals, is its
/// interior.
#[expect(clippy::cast_possible_truncation)]
#[expect(clippy::cast_precision_loss)]
fn emit_runs<R: Runs + ?Sized, F: FnMut(usize, std::ops::Range<usize>, &mut Vec<u8>)>(
    runs: &mut R,
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
    let mut at = runs.first();
    while let Some(head) = at {
        let (start, head_end) = runs.columns(head);
        match runs.mark(head).link() {
            Link::Skip => {}
            Link::Span => out.spans.push(rect(start, head_end)),
            Link::Head => {
                let mut last = head;
                let mut interior = 0..0;
                loop {
                    let columns = runs.columns(last);
                    if runs.mark(last).class() == Class::Full
                        && columns.1 - columns.0 > interior.end - interior.start
                    {
                        interior = columns.0..columns.1;
                    }
                    match runs.next(last) {
                        Some(next) if runs.mark(next).link() == Link::Cell => last = next,
                        _ => break,
                    }
                }
                let end = runs.columns(last).1;
                let interior = if interior.is_empty() {
                    0..0
                } else {
                    (interior.start - start) as usize..(interior.end - start) as usize
                };
                make_cell(
                    append_row,
                    start as usize,
                    rect(start, end),
                    (end - start) as usize,
                    sh,
                    interior,
                    &mut out.cells,
                    rasters,
                    arena,
                    reserve,
                )?;
                at = runs.next(last);
                continue;
            }
            Link::Cell => unreachable!("a cell's runs follow its head"),
        }
        at = runs.next(head);
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
    Empty = 0,
    /// Every texel is `u8::MAX`.
    Full = 1,
    /// Anything else.
    Partial = 2,
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
/// every chosen cell `Head` and the rest of it `Cell`, so the emission
/// walk reads each cell as a `Head` and the `Cell` runs after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Link {
    /// The run is empty and emits nothing.
    Skip = 0,
    /// The run is full and emits a span.
    Span = 1,
    /// The run is part of a cell that starts at an earlier run.
    Cell = 2,
    /// The run starts a cell.
    Head = 3,
}

/// A run's state in five bits: its [`Class`] in bits 0–1, its [`Link`]
/// in bits 2–3 and [`Mark::OPENS`] in bit 4.
#[derive(Clone, Copy, Debug)]
struct Mark(u8);

impl Mark {
    /// The bits a mark uses.
    const BITS: u32 = 5;
    /// Set by [`partition`] on a run that became the cheapest cell start
    /// so far: the cell that ends at any later run starts at the nearest
    /// run carrying it.
    const OPENS: u8 = 1 << 4;

    /// A run of `class` that the partition has not reached.
    const fn new(class: Class) -> Self {
        Self(class as u8)
    }

    /// The run's class.
    const fn class(self) -> Class {
        match self.0 & 0b11 {
            0 => Class::Empty,
            1 => Class::Full,
            _ => Class::Partial,
        }
    }

    /// What the run became; `Skip` until [`partition`] sets it.
    const fn link(self) -> Link {
        match self.0 >> 2 & 0b11 {
            0 => Link::Skip,
            1 => Link::Span,
            2 => Link::Cell,
            _ => Link::Head,
        }
    }

    /// The mark with `link`, keeping its class and flag.
    const fn with_link(self, link: Link) -> Self {
        Self(self.0 & !0b1100 | (link as u8) << 2)
    }

    /// Whether the mark carries [`Mark::OPENS`].
    const fn opens(self) -> bool {
        self.0 & Self::OPENS != 0
    }

    /// The mark with [`Mark::OPENS`].
    const fn with_opens(self) -> Self {
        Self(self.0 | Self::OPENS)
    }
}

/// A band's class runs, maximal ranges of equal-class columns, as
/// [`partition`] and [`emit_runs`] walk them. `At` names one run.
trait Runs {
    /// A run's position.
    type At: Copy + PartialEq;

    /// The band's first run.
    fn first(&self) -> Option<Self::At>;
    /// The band's last run.
    fn last(&self) -> Option<Self::At>;
    /// The run after `at`.
    fn next(&self, at: Self::At) -> Option<Self::At>;
    /// The run before `at`.
    fn prev(&self, at: Self::At) -> Option<Self::At>;
    /// The run's columns, `(start, end)`.
    fn columns(&self, at: Self::At) -> (u32, u32);
    /// The run's mark.
    fn mark(&self, at: Self::At) -> Mark;
    /// Replaces the run's mark; its class stays.
    fn set_mark(&mut self, at: Self::At, mark: Mark);
}

/// A sparse band's run, ending at column [`Run::end`]: it starts where
/// the run before it ends, or at column 0. The record is one word, the
/// end column above the run's [`Mark`].
#[derive(Clone, Copy, Debug)]
struct Run(u32);

impl Run {
    /// The record of a run ending at `end` with `mark`.
    fn new(end: u32, mark: Mark) -> Self {
        assert!(
            end < 1 << (u32::BITS - Mark::BITS),
            "a band's columns fit a run's end"
        );
        Self(end << Mark::BITS | u32::from(mark.0))
    }

    /// The column the run ends at.
    const fn end(self) -> u32 {
        self.0 >> Mark::BITS
    }

    /// The run's mark.
    #[expect(clippy::cast_possible_truncation, reason = "the mark is the low bits")]
    const fn mark(self) -> Mark {
        Mark(self.0 as u8 & ((1 << Mark::BITS) - 1))
    }
}

/// One record per run, `At` its index.
impl Runs for [Run] {
    type At = usize;

    fn first(&self) -> Option<usize> {
        (!self.is_empty()).then_some(0)
    }

    fn last(&self) -> Option<usize> {
        self.len().checked_sub(1)
    }

    fn next(&self, at: usize) -> Option<usize> {
        (at + 1 < self.len()).then_some(at + 1)
    }

    fn prev(&self, at: usize) -> Option<usize> {
        at.checked_sub(1)
    }

    fn columns(&self, at: usize) -> (u32, u32) {
        (
            at.checked_sub(1).map_or(0, |prev| self[prev].end()),
            self[at].end(),
        )
    }

    fn mark(&self, at: usize) -> Mark {
        self[at].mark()
    }

    fn set_mark(&mut self, at: usize, mark: Mark) {
        self[at] = Run::new(self[at].end(), mark);
    }
}

/// A dense band's columns, one byte each: every column holds its class,
/// and a run's first column holds the run's [`Mark`]. `At` is the run's
/// `(start, end)` columns, and stepping scans to the next class change.
/// A dense band already pays a byte per column to classify, so its runs
/// cost no more than the columns themselves.
struct Columns<'a>(&'a mut [u8]);

impl Columns<'_> {
    /// The class of column `x`.
    const fn class(&self, x: usize) -> u8 {
        self.0[x] & 0b11
    }

    /// The end of the run starting at column `start`.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a band is at most a surface wide"
    )]
    fn end_of(&self, start: u32) -> u32 {
        let class = self.class(start as usize);
        let rest = &self.0[start as usize + 1..];
        start + 1 + rest.iter().take_while(|&&c| c & 0b11 == class).count() as u32
    }

    /// The start of the run ending at column `end`.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a band is at most a surface wide"
    )]
    fn start_of(&self, end: u32) -> u32 {
        let class = self.class(end as usize - 1);
        let before = &self.0[..end as usize - 1];
        end - 1
            - before
                .iter()
                .rev()
                .take_while(|&&c| c & 0b11 == class)
                .count() as u32
    }
}

impl Runs for Columns<'_> {
    type At = (u32, u32);

    fn first(&self) -> Option<(u32, u32)> {
        (!self.0.is_empty()).then(|| (0, self.end_of(0)))
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "a band is at most a surface wide"
    )]
    fn last(&self) -> Option<(u32, u32)> {
        let end = self.0.len() as u32;
        (end > 0).then(|| (self.start_of(end), end))
    }

    fn next(&self, (_, end): (u32, u32)) -> Option<(u32, u32)> {
        ((end as usize) < self.0.len()).then(|| (end, self.end_of(end)))
    }

    fn prev(&self, (start, _): (u32, u32)) -> Option<(u32, u32)> {
        (start > 0).then(|| (self.start_of(start), start))
    }

    fn columns(&self, at: (u32, u32)) -> (u32, u32) {
        at
    }

    fn mark(&self, (start, _): (u32, u32)) -> Mark {
        Mark(self.0[start as usize])
    }

    fn set_mark(&mut self, (start, _): (u32, u32), mark: Mark) {
        self.0[start as usize] = mark.0;
    }
}

/// Appends the columns up to `end` as `class`, extending the last run
/// when it has the same class.
fn push_run(runs: &mut Vec<Run>, end: u32, class: Class) {
    match runs.last_mut() {
        Some(run) if run.mark().class() == class => *run = Run::new(end, run.mark()),
        _ => runs.push(Run::new(end, Mark::new(class))),
    }
}

/// Classifies each column of a dense band of `w` columns and `sh` rows
/// into `columns`.
fn dense_columns(strip: &[f32], w: usize, sh: usize, columns: &mut Vec<u8>) {
    columns.clear();
    columns.resize(w, 0);
    for (x, column) in columns.iter_mut().enumerate() {
        let mut state = (true, true);
        for row in 0..sh {
            state = Class::fold(state, strip[row * w + x]);
        }
        *column = Mark::new(Class::new(state.0, state.1)).0;
    }
}

/// Splits a sparse band into class runs. Each row's constant-coverage
/// runs end at column `w`; the band's columns split wherever any row's
/// run ends, and every column between two splits shares one class.
fn sparse_runs(coverage: &[(u32, f32)], ends: &[usize], w: usize, runs: &mut Vec<Run>) {
    runs.clear();
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
        push_run(runs, next, Class::new(state.0, state.1));
        pos = next;
        for cursor in cursors.iter_mut() {
            if coverage[*cursor].0 == next {
                *cursor += 1;
            }
        }
    }
}

/// What the partition charges for one instance, as a multiple of
/// `size_of::<Instance>()` (numerator, denominator), against one byte per
/// stored texel. It trades two per-frame costs: every instance is
/// replayed into the frame's instance list, and every stored texel —
/// a bridged gap's zeros included — is a fragment the GPU shades. Too
/// light, a band splits into more instances than replay can afford; too
/// heavy, cells bridge gaps wide enough to cost GPU time. 3.5 is where
/// both stay at or below the classification it replaces.
const INSTANCE_WEIGHT: (u32, u32) = (7, 2);

/// The byte cost the partition charges for one instance.
#[expect(
    clippy::cast_possible_truncation,
    reason = "an instance is a few hundred bytes"
)]
const INSTANCE_BYTES: u32 = size_of::<Instance>() as u32 * INSTANCE_WEIGHT.0 / INSTANCE_WEIGHT.1;

/// Chooses the band's emission with the least byte cost: every instance
/// costs [`INSTANCE_BYTES`], and every texel a cell stores its byte in
/// the atlas, so a cell costs `instance + sh · width`. An empty gap
/// emits nothing and a full run outside a cell is a span, so a gap is
/// bridged, and a full run carried as texels, exactly when that is
/// cheaper than the instances a split would add.
///
/// A cell from run `j` through run `k` costs `before_j − sh · start_j +
/// instance + sh · end_k`, so the cheapest cell ending at `k` starts at
/// the running minimum of `before_j − sh · start_j` over non-empty runs,
/// and one pass solves the band. Each run that lowers the minimum
/// carries [`Mark::OPENS`], so a cell ending at `k` starts at the
/// nearest such run at or before `k`; the backward pass marks each
/// emitted cell from there.
fn partition<R: Runs + ?Sized>(runs: &mut R, sh: usize) {
    let sh = i64::from(u32::try_from(sh).expect("a band is at most STRIP_H rows"));
    let instance = i64::from(INSTANCE_BYTES);
    // The cheapest cell start so far, as `before_j − sh · start_j`.
    let mut open: Option<i64> = None;
    // The least byte cost of emitting every run before the current one.
    let mut before = 0;
    let mut at = runs.first();
    while let Some(run) = at {
        let (start, end) = runs.columns(run);
        let mut mark = runs.mark(run);
        let (cost, link) = if mark.class() == Class::Empty {
            // A cell never starts or ends on a gap.
            (before, Link::Skip)
        } else {
            let here = before - sh * i64::from(start);
            let best = match open {
                Some(best) if best <= here => best,
                _ => {
                    mark = mark.with_opens();
                    *open.insert(here)
                }
            };
            let cell = best + instance + sh * i64::from(end);
            let span = before + instance;
            if mark.class() == Class::Full && span <= cell {
                (span, Link::Span)
            } else {
                (cell, Link::Cell)
            }
        };
        runs.set_mark(run, mark.with_link(link));
        before = cost;
        at = runs.next(run);
    }
    let mut at = runs.last();
    while let Some(last) = at {
        let link = runs.mark(last).link();
        at = match link {
            Link::Skip | Link::Span => runs.prev(last),
            Link::Cell => {
                let mut from = last;
                while !runs.mark(from).opens() {
                    from = runs
                        .prev(from)
                        .expect("a cell ends at or after the run that opened it");
                }
                runs.set_mark(from, runs.mark(from).with_link(Link::Head));
                let mut run = from;
                while run != last {
                    run = runs.next(run).expect("a cell's runs reach its last");
                    runs.set_mark(run, runs.mark(run).with_link(Link::Cell));
                }
                runs.prev(from)
            }
            Link::Head => unreachable!("the forward pass marks no heads"),
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

/// Each run's end column and class, in order.
#[cfg(test)]
fn classes<R: Runs + ?Sized>(runs: &R) -> Vec<(u32, Class)> {
    let mut out = Vec::new();
    let mut at = runs.first();
    while let Some(run) = at {
        out.push((runs.columns(run).1, runs.mark(run).class()));
        at = runs.next(run);
    }
    out
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

    /// An empty gap is carried as zero texels while they cost fewer bytes
    /// than the instance a second cell would, and splits the cell into
    /// two, drawing nothing between them, when they cost more.
    #[test]
    fn a_gap_is_bridged_while_its_texels_cost_less_than_an_instance() {
        let g = INSTANCE_BYTES as usize / STRIP_H;
        let mut arena = Vec::new();
        for (gap, cells) in [(g - 1, 1), (g + 1, 2)] {
            let (emitted, rasters) =
                emit(&strip(&[(0, 3), (3 + gap, 6 + gap)], &[], 2000), &mut arena).unwrap();
            assert_eq!((emitted.cells.len(), emitted.spans.len()), (cells, 0));
            if cells == 1 {
                let stored = 6 + gap;
                assert_eq!(rasters[0].0 as usize, stored);
                for row in arena[rasters[0].2.clone()].chunks_exact(stored) {
                    assert!(
                        row[3..3 + gap].iter().all(|&t| t == 0),
                        "the carried gap is zero coverage"
                    );
                }
            } else {
                assert_eq!(rasters.iter().map(|r| r.0).collect::<Vec<_>>(), [3, 3]);
            }
        }
    }

    /// A full run between partial runs is carried as texels, and is the
    /// cell's interior, while they cost fewer bytes than the two instances
    /// a split adds; wider, it is a span between two cells. A full run
    /// with no partial neighbour is a span.
    #[test]
    fn a_full_run_splits_a_cell_only_when_its_texels_cost_more() {
        let limit = 2 * INSTANCE_BYTES as usize / STRIP_H;
        let mut arena = Vec::new();
        for (f, cells, spans) in [(limit - 6, 1, 0), (limit + 6, 2, 1)] {
            let (emitted, _) = emit(
                &strip(&[(0, 3), (3 + f, 6 + f)], &[(3, 3 + f)], 2000),
                &mut arena,
            )
            .unwrap();
            assert_eq!((emitted.cells.len(), emitted.spans.len()), (cells, spans));
            if cells == 1 {
                assert_eq!(
                    emitted.cells[0].interior,
                    3 | u32::try_from(3 + f).unwrap() << 16
                );
            } else {
                assert_eq!(
                    emitted.spans[0][2] - emitted.spans[0][0],
                    f32::from(u16::try_from(f).unwrap())
                );
                assert!(emitted.cells.iter().all(|cell| cell.interior == 0));
            }
        }
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
        let mut columns = Vec::new();
        dense_columns(&coverage.data, coverage.w, STRIP_H, &mut columns);
        assert_eq!(
            classes(&Columns(&mut columns)),
            [(10, Class::Empty), (13, Class::Partial), (40, Class::Empty)]
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
            dense_columns(&strip, w, sh, &mut dense);
            sparse_runs(&runs, &ends, w, &mut sparse);
            assert_eq!(classes(&Columns(&mut dense)), classes(sparse.as_slice()));
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
