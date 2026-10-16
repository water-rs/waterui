//! CPU coverage rasterization of general paths: sparse strips into atlas
//! cells, plus the cache key for replaying them.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use cherenkov::{FillRule, ShapeData};
use kurbo::{Affine, BezPath, PathEl, Point, Rect, Shape as _, Vec2};

use crate::render::glyph::{CellTexels, PathCell, PathEmit};
#[cfg(test)]
use crate::render::raster::Raster;
use crate::render::raster::{Band, BandRaster};
use cherenkov::RenderError;

/// Coverage strip height in device rows.
pub const STRIP_H: usize = 4;

/// A run of full columns shorter than this is emitted as a cell rather than
/// splitting a partial run.
const FULL_RUN_MIN: usize = 8;

/// Empty-column gaps up to this width between two partial runs are carried as
/// zero texels inside one cell rather than as a separate cell instance.
const PARTIAL_GAP_MAX: usize = 8;

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
    /// The GPU compute route of water-rs/waterui#2543: curve elements go
    /// to the frame's compute input buffer untouched by the CPU, and the
    /// coverage and classification kernels emit the same spans and
    /// partial cells. Route selection is water-rs/waterui#2545; in this
    /// change the route is reachable only through the test hook.
    Gpu,
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
    sweep(segments, bbox, surface, rule, &mut scratch.raster)
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
/// column classification. The owner keeps one across paths and frames so
/// emission allocates only for growth.
#[derive(Debug, Default)]
pub struct EmitScratch {
    raster: BandRaster,
    /// `strip_class`'s per-column classes for a dense band.
    class: Vec<u8>,
    /// `strip_class_sparse`'s `(end, class)` intervals for a sparse band.
    intervals: Vec<(u32, u8)>,
}

impl EmitScratch {
    /// Empty buffers; the first emissions size them.
    pub const fn new() -> Self {
        Self {
            raster: BandRaster::new(),
            class: Vec::new(),
            intervals: Vec::new(),
        }
    }

    /// Heap bytes the buffers hold between emissions.
    pub const fn heap_bytes(&self) -> u64 {
        self.raster.heap_bytes()
            + (self.class.capacity() + self.intervals.capacity() * size_of::<(u32, u8)>()) as u64
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
    let (x0, y0, w, h, clipped) = grid(bbox, surface)?;
    let (segments, rule) = resolve(segments, rule);
    let EmitScratch {
        raster,
        class,
        intervals,
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
                class,
                &mut out,
                &mut rasters,
                arena,
                &mut reserve,
            ),
            Band::Sparse(runs, ends) => emit_strip_sparse(
                runs,
                ends,
                w,
                x0,
                y0,
                sy,
                intervals,
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
    let resolved = cherenkov::lowering::resolve_winding(segments, rule).into_resolved();
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

/// One curve element encoded for the coverage kernel: three `vec4`s in
/// the compute input buffer — `(kind, n, path index, segment slot)`, the
/// start point and first control point, and the (padded) second control
/// point and end point. `n` is the uniform subdivision count
/// [`wang_segments`] picked, mirrored exactly by the flatten kernel.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuElement {
    /// `(kind, n, path index, segment slot)` as exact integers.
    pub head: [u32; 4],
    /// Start point and first control point.
    pub lo: [f32; 4],
    /// Second control point (padded with the first for a quad, unused
    /// for a line) and the end point.
    pub hi: [f32; 4],
}

/// Decision R2's per-path buffer bound, computed in `O(n)` over the
/// curve elements from their device-space bounding boxes.
#[derive(Clone, Copy, Debug, Default)]
pub struct GpuBound {
    /// Segments the flatten kernel will produce.
    pub segments: usize,
    /// Segment/strip incidences bounded by vertical control-polygon variation.
    pub bins: usize,
    /// Segment/cell incidences bounded by control-polygon L1 variation.
    pub fragments: usize,
    /// Logical key stride; the kernel's tighter grid fits within it.
    pub grid_w: usize,
    /// Bound raster grid rows.
    pub grid_h: usize,
    /// Maximum span instances the path can emit.
    pub span_cap: usize,
    /// Maximum cell instances the path can emit.
    pub cell_cap: usize,
    /// Stored R8 bytes, excluding implicit full interiors.
    pub texels: usize,
    /// The bound grid is empty: the path cannot emit anything.
    pub empty: bool,
    /// Conservative device bounds, used by pass planning before compute.
    pub rect: [f32; 4],
}

/// The uniform subdivision count for `kind` (1 quad, 2 cubic) whose
/// control hull sits at `points` in device space, at the `FLATTEN`
/// tolerance. The kernel subdivides uniformly into exactly this many
/// segments; the same closed form runs here so the segment slots match.
#[expect(
    clippy::cast_possible_truncation,
    reason = "subdivision counts are small"
)]
#[expect(clippy::cast_sign_loss)]
fn wang_segments(kind: u32, p0: Point, a: Point, b: Point, p1: Point) -> usize {
    let second = match kind {
        0 => return 1,
        1 => (p0.to_vec2() - a.to_vec2() * 2.0 + p1.to_vec2()).hypot() * 2.0,
        _ => {
            (p0.to_vec2() - a.to_vec2() * 2.0 + b.to_vec2())
                .hypot()
                .max((a.to_vec2() - b.to_vec2() * 2.0 + p1.to_vec2()).hypot())
                * 6.0
        }
    };
    // Linear interpolation error <= max|B''| / (8 n^2).
    (second / (8.0 * FLATTEN)).sqrt().ceil().max(1.0) as usize
}

/// The element walk state of [`gpu_bound`]: the encoded records so far,
/// the running bounds and the subpath cursor.
struct GpuEncode {
    out: Vec<GpuElement>,
    last: Point,
    start: Point,
    raster: Affine,
    bound: GpuBound,
}

impl GpuEncode {
    /// Encodes one producing element — a line, quad or cubic from the
    /// cursor's `last` to `p1` — into a [`GpuElement`] and adds its
    /// device bbox to the hull and touched-cell bound.
    #[expect(clippy::cast_possible_truncation, reason = "device coords are f32")]
    #[expect(
        clippy::cast_sign_loss,
        reason = "absolute control-polygon variation is nonnegative"
    )]
    fn push(&mut self, hull: &mut Rect, kind: u32, a: Point, b: Point, p1: Point) {
        if kind == 0 && p1 == self.last {
            return;
        }
        // The subdivision count is measured on the device-space control
        // hull; the element itself is uploaded in local space for the
        // kernel to transform.
        let n = wang_segments(
            kind,
            self.raster * self.last,
            self.raster * a,
            self.raster * b,
            self.raster * p1,
        );
        let mut dev = Rect::from_points(self.raster * self.last, self.raster * p1);
        dev = dev.union_pt(self.raster * a);
        if kind == 2 {
            dev = dev.union_pt(self.raster * b);
        }
        *hull = hull.union(dev);
        let points = [self.last, a, b, p1];
        let short;
        let points: &[Point] = match kind {
            0 => {
                short = [self.last, p1, p1];
                &short[..2]
            }
            1 => {
                short = [self.last, a, p1];
                &short
            }
            _ => &points,
        };
        let mut variation = kurbo::Vec2::ZERO;
        for pair in points.windows(2) {
            let delta = self.raster * pair[1] - self.raster * pair[0];
            variation.x += delta.x.abs();
            variation.y += delta.y.abs();
        }
        // Each chord visits at most |dx|+|dy|+3 cells, including
        // horizontal/vertical boundary ownership. Bezier variation is
        // bounded by its control polygon's variation.
        self.bound.bins += n * 2
            + (variation.y / f64::from(u32::try_from(STRIP_H).expect("strip height fits u32")))
                .ceil() as usize;
        self.bound.fragments += n * 3 + (variation.x + variation.y).ceil() as usize;
        self.out.push(GpuElement {
            head: [
                kind,
                u32::try_from(n).expect("subdivision count"),
                0,
                u32::try_from(self.bound.segments).expect("segment count"),
            ],
            lo: [
                self.last.x as f32,
                self.last.y as f32,
                a.x as f32,
                a.y as f32,
            ],
            hi: [b.x as f32, b.y as f32, p1.x as f32, p1.y as f32],
        });
        self.bound.segments += n;
        self.last = p1;
    }
}

/// Encodes `elements` (the local path elements, already closed through
/// `ClosePath`/`MoveTo` semantics) for the compute input buffer under
/// the device transform `raster`, and computes the R2 bound over their
/// device bounding boxes. Returns the element records (their `path` and
/// `slot` fields zeroed — the caller stamps them) and the bound.
#[expect(clippy::cast_possible_truncation, reason = "device coords are f32")]
#[expect(
    clippy::cast_sign_loss,
    reason = "grid dimensions are clamped nonnegative"
)]
pub fn gpu_bound(
    elements: &[PathEl],
    raster: Affine,
    surface: (f64, f64),
) -> (Vec<GpuElement>, GpuBound) {
    let clip = Rect::new(0.0, 0.0, surface.0, surface.1);
    let mut hull = Rect::new(f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    let mut state = GpuEncode {
        out: Vec::new(),
        last: Point::ORIGIN,
        start: Point::ORIGIN,
        raster,
        bound: GpuBound::default(),
    };
    for &el in elements {
        match el {
            PathEl::MoveTo(p) => {
                state.push(&mut hull, 0, state.last, state.last, state.start);
                state.start = p;
                state.last = p;
            }
            PathEl::LineTo(p) => {
                let p0 = state.last;
                state.push(&mut hull, 0, p0, p0, p);
            }
            PathEl::QuadTo(c, p) => {
                state.push(&mut hull, 1, c, c, p);
            }
            PathEl::CurveTo(c0, c1, p) => {
                state.push(&mut hull, 2, c0, c1, p);
            }
            PathEl::ClosePath => {
                let p0 = state.last;
                let s = state.start;
                state.push(&mut hull, 0, p0, p0, s);
                state.last = s;
            }
        }
    }
    {
        let (p0, s) = (state.last, state.start);
        state.push(&mut hull, 0, p0, p0, s);
    }
    let (out, mut bound) = (state.out, state.bound);
    // The bound raster grid: the hull inflated by a pixel and clipped to
    // the surface, floored and ceiled like the sweep's rect.
    let inflated = hull.inflate(1.0, 1.0);
    let r = inflated.intersect(clip);
    let w = (r.x1.ceil() - r.x0.floor()).max(0.0) as usize;
    let h = (r.y1.ceil() - r.y0.floor()).max(0.0) as usize;
    bound.rect = [
        r.x0.floor() as f32,
        r.y0.floor() as f32,
        r.x1.ceil() as f32,
        r.y1.ceil() as f32,
    ];
    bound.grid_w = w;
    bound.grid_h = h;
    // Partial columns, absorbed gaps (<=8) and demoted full runs (<8)
    // need at most 16 columns per touched boundary column. Four rows
    // per strip; the retained large full interval is implicit.
    bound.texels = if w <= 32 && h <= 32 {
        w * h
    } else {
        bound.fragments * 64
    };
    bound.empty = w == 0 || h == 0;
    // Cells are column-disjoint inside their strip, so the strip count
    // times the grid width is the grid-side cap; spans obey the same
    // bound. The element bound may be tighter.
    let strip_cap = h.div_ceil(STRIP_H) * w;
    bound.cell_cap = bound
        .fragments
        .min(strip_cap)
        .max(usize::from(w <= 32 && h <= 32));
    bound.span_cap = bound.cell_cap + h.div_ceil(STRIP_H);
    (out, bound)
}

/// Emits `coverage` as spans and atlas cells: one cell when the grid is
/// small, otherwise strips of [`STRIP_H`] rows split into full-column span
/// runs and partial-column cells.
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
                append_dense_row(&coverage.data[r * coverage.w..][columns], arena);
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
        let mut class = Vec::new();
        for sy in (0..coverage.h).step_by(STRIP_H) {
            let sh = STRIP_H.min(coverage.h - sy);
            emit_strip(
                &coverage.data[sy * coverage.w..(sy + sh) * coverage.w],
                coverage.w,
                coverage.x,
                coverage.y,
                sy,
                sh,
                &mut class,
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
/// with `rect` in device space. `reserve` is the emission's pending
/// reservation: its grid area before the first cell, zero after.
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

/// Emits one band's runs: all-full runs become spans, partial runs
/// become cells cut out of `strip` — `sh * w` coverage values for the
/// `sy`-anchored band of a grid at `cov_x, cov_y`. `class` and `arena`
/// are the emission's classification and texel scratch, `reserve` its
/// pending reservation (see [`make_cell`]).
#[expect(clippy::cast_possible_truncation)]
#[expect(clippy::cast_precision_loss)]
#[expect(clippy::too_many_arguments)]
fn emit_strip(
    strip: &[f32],
    w: usize,
    cov_x: f64,
    cov_y: f64,
    sy: usize,
    sh: usize,
    class: &mut Vec<u8>,
    out: &mut PathEmit,
    rasters: &mut Vec<CellTexels>,
    arena: &mut Vec<u8>,
    reserve: &mut usize,
) -> Result<(), RenderError> {
    strip_class(strip, w, sh, class);
    // Emit runs: all-full runs become spans, the rest cells.
    let mut x = 0;
    while x < w {
        if class[x] == 0 {
            x += 1;
            continue;
        }
        let start = x;
        if class[x] == 1 {
            while x < w && class[x] == 1 {
                x += 1;
            }
            out.spans.push([
                (cov_x + start as f64) as f32,
                (cov_y + sy as f64) as f32,
                (cov_x + x as f64) as f32,
                (cov_y + (sy + sh) as f64) as f32,
            ]);
        } else {
            while x < w && class[x] == 2 {
                x += 1;
            }
            let full_start = x;
            while x < w && class[x] == 1 {
                x += 1;
            }
            let full_end = x;
            while x < w && class[x] == 2 {
                x += 1;
            }
            let interior = if full_start == full_end {
                0..0
            } else {
                full_start - start..full_end - start
            };
            make_cell(
                &mut |r, columns, arena| append_dense_row(&strip[r * w..][columns], arena),
                start,
                [
                    (cov_x + start as f64) as f32,
                    (cov_y + sy as f64) as f32,
                    (cov_x + x as f64) as f32,
                    (cov_y + (sy + sh) as f64) as f32,
                ],
                x - start,
                sh,
                interior,
                &mut out.cells,
                rasters,
                arena,
                reserve,
            )?;
        }
    }
    Ok(())
}

/// Quantize once, identically for dense cells and sparse coverage runs.
#[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn quantize(coverage: f32) -> u8 {
    (coverage.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn append_dense_row(row: &[f32], arena: &mut Vec<u8>) {
    arena.extend(row.iter().copied().map(quantize));
}

/// `strip_class`'s sparse twin: the strip arrives as `(end_column,
/// coverage)` runs per row, so boundary columns merge into `(end,
/// class)` column intervals — every column in an interval shares its
/// class — and the same run rules apply to interval widths.
#[expect(clippy::cast_possible_truncation)]
#[expect(
    clippy::float_cmp,
    reason = "a column is 'full' exactly when coverage clamped to 1.0"
)]
fn strip_class_sparse(
    runs: &[(u32, f32)],
    ends: &[usize],
    w: usize,
    intervals: &mut Vec<(u32, u8)>,
) {
    intervals.clear();
    // `ends[r]` bounds row `r`'s runs; the cursors index the flat
    // buffer absolutely and a row's runs always end at column `w`, so
    // no cursor can read past its row.
    let mut cursors = [0usize; STRIP_H];
    let mut base = 0usize;
    for (c, &end) in cursors.iter_mut().zip(ends.iter()) {
        *c = base;
        base = end;
    }
    let n = ends.len();
    let mut pos = 0u32;
    while pos < w as u32 {
        let mut next = w as u32;
        let mut all_zero = true;
        let mut all_one = true;
        for &c in &cursors[..n] {
            let (end, cov) = runs[c];
            next = next.min(end);
            all_zero &= cov == 0.0;
            all_one &= cov == 1.0;
        }
        let class = if all_zero {
            0
        } else if all_one {
            1
        } else {
            2
        };
        if intervals.last().is_some_and(|&(_, c)| c == class) {
            if let Some(last) = intervals.last_mut() {
                last.0 = next;
            }
        } else {
            intervals.push((next, class));
        }
        pos = next;
        for c in &mut cursors[..n] {
            if runs[*c].0 == next {
                *c += 1;
            }
        }
    }
    // Full runs under `FULL_RUN_MIN` demote to partial, then empty gaps
    // up to `PARTIAL_GAP_MAX` between partials join them.
    for i in 0..intervals.len() {
        let start = if i == 0 { 0 } else { intervals[i - 1].0 };
        let (end, class) = intervals[i];
        if class == 1 && end - start < FULL_RUN_MIN as u32 {
            intervals[i].1 = 2;
        }
    }
    for i in 1..intervals.len() - 1 {
        if intervals[i].1 == 0
            && intervals[i - 1].1 == 2
            && intervals[i + 1].1 == 2
            && intervals[i].0 - intervals[i - 1].0 <= PARTIAL_GAP_MAX as u32
        {
            intervals[i].1 = 2;
        }
    }
    // Demotion and gap absorption create adjacent partial intervals.
    // Coalesce once, in place: repeatedly draining a gap would shift the
    // remaining intervals on every merge and make the pass quadratic.
    let mut written = 0;
    for read in 0..intervals.len() {
        let interval = intervals[read];
        if written > 0 && intervals[written - 1].1 == interval.1 {
            intervals[written - 1].0 = interval.0;
        } else {
            intervals[written] = interval;
            written += 1;
        }
    }
    intervals.truncate(written);
}

/// `emit_strip`'s sparse twin: `(end, class)` intervals replace the
/// per-column class array; only partial cells materialize R8 texels.
#[expect(clippy::cast_possible_truncation)]
#[expect(clippy::cast_precision_loss)]
#[expect(clippy::too_many_arguments)]
fn emit_strip_sparse(
    runs: &[(u32, f32)],
    ends: &[usize],
    w: usize,
    cov_x: f64,
    cov_y: f64,
    sy: usize,
    intervals: &mut Vec<(u32, u8)>,
    out: &mut PathEmit,
    rasters: &mut Vec<CellTexels>,
    arena: &mut Vec<u8>,
    reserve: &mut usize,
) -> Result<(), RenderError> {
    let sh = ends.len();
    strip_class_sparse(runs, ends, w, intervals);
    let mut prev = 0u32;
    let mut i = 0;
    while i < intervals.len() {
        let start = prev;
        let (end, class) = intervals[i];
        prev = end;
        i += 1;
        if class == 0 {
            continue;
        }
        if class == 1 {
            out.spans.push([
                (cov_x + f64::from(start)) as f32,
                (cov_y + sy as f64) as f32,
                (cov_x + f64::from(end)) as f32,
                (cov_y + (sy + sh) as f64) as f32,
            ]);
            continue;
        }
        let cell_start = start;
        let mut cell_end = end;
        while i < intervals.len() && intervals[i].1 == 2 {
            cell_end = intervals[i].0;
            prev = cell_end;
            i += 1;
        }
        let mut full = cell_end..cell_end;
        if i < intervals.len() && intervals[i].1 == 1 {
            full = prev..intervals[i].0;
            cell_end = intervals[i].0;
            prev = cell_end;
            i += 1;
            while i < intervals.len() && intervals[i].1 == 2 {
                cell_end = intervals[i].0;
                prev = cell_end;
                i += 1;
            }
        }
        let interior = if full.is_empty() {
            0..0
        } else {
            usize::try_from(full.start - cell_start).expect("interior start fits usize")
                ..usize::try_from(full.end - cell_start).expect("interior end fits usize")
        };
        make_cell(
            &mut |r, columns, arena| {
                let row = &runs[if r == 0 { 0 } else { ends[r - 1] }..ends[r]];
                let first = row.partition_point(|&(end, _)| end as usize <= columns.start);
                let mut x = columns.start;
                for &(end, coverage) in &row[first..] {
                    let end = (end as usize).min(columns.end);
                    arena.resize(arena.len() + end - x, quantize(coverage));
                    x = end;
                    if x == columns.end {
                        break;
                    }
                }
            },
            usize::try_from(cell_start).expect("cell start fits usize"),
            [
                (cov_x + f64::from(cell_start)) as f32,
                (cov_y + sy as f64) as f32,
                (cov_x + f64::from(cell_end)) as f32,
                (cov_y + (sy + sh) as f64) as f32,
            ],
            usize::try_from(cell_end - cell_start).expect("cell width fits usize"),
            sh,
            interior,
            &mut out.cells,
            rasters,
            arena,
            reserve,
        )?;
    }
    Ok(())
}

/// Classifies one strip's columns as 0 empty, 1 full (1.0 in every row),
/// 2 partial. Full runs narrower than `FULL_RUN_MIN` are demoted to
/// partial so a partial run is not fragmented by isolated full columns,
/// and empty gaps up to `PARTIAL_GAP_MAX` between two partial runs are
/// carried as zero texels inside one cell rather than as a separate cell
/// instance.
#[expect(
    clippy::float_cmp,
    reason = "a column is 'full' exactly when coverage clamped to 1.0"
)]
fn strip_class(strip: &[f32], w: usize, sh: usize, class: &mut Vec<u8>) {
    class.clear();
    class.resize(w, 0);
    for (x, c) in class.iter_mut().enumerate() {
        let mut empty = true;
        let mut full = true;
        for row in 0..sh {
            let v = strip[row * w + x];
            empty &= v == 0.0;
            full &= v == 1.0;
        }
        *c = if empty {
            0
        } else if full {
            1
        } else {
            2
        };
    }
    let mut x = 0;
    while x < w {
        if class[x] == 1 {
            let end = (x + 1..w).find(|&i| class[i] != 1).unwrap_or(w);
            if end - x < FULL_RUN_MIN {
                class[x..end].fill(2);
            }
            x = end;
        } else {
            x += 1;
        }
    }
    let mut x = 0;
    while x < w {
        if class[x] == 0 {
            let end = (x + 1..w).find(|&i| class[i] != 0).unwrap_or(w);
            if x > 0
                && end < w
                && class[x - 1] == 2
                && class[end] == 2
                && end - x <= PARTIAL_GAP_MAX
            {
                class[x..end].fill(2);
            }
            x = end;
        } else {
            x += 1;
        }
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

    /// Empty gaps up to `PARTIAL_GAP_MAX` between partial columns are
    /// carried inside one cell; wider gaps, and gaps next to a full run,
    /// still split.
    #[test]
    fn short_empty_gaps_merge_into_one_cell() {
        let cov = |partial: &[(usize, usize)], full: &[(usize, usize)], w: usize, h: usize| {
            let mut data = vec![0.0f32; w * h];
            for row in 0..h {
                for &(a, b) in partial {
                    data[row * w + a..row * w + b].fill(0.5);
                }
                for &(a, b) in full {
                    data[row * w + a..row * w + b].fill(1.0);
                }
            }
            Coverage {
                x: 0.0,
                y: 0.0,
                w,
                h,
                clipped: false,
                data,
            }
        };
        // Gap of `PARTIAL_GAP_MAX` between two partial runs: one cell.
        let g = PARTIAL_GAP_MAX.max(1);
        let mut arena = Vec::new();
        let (emitted, rasters) =
            emit(&cov(&[(0, 3), (3 + g, 6 + g)], &[], 40, 4), &mut arena).unwrap();
        assert_eq!(emitted.cells.len(), 1);
        assert_eq!(rasters[0].0 as usize, 6 + g);
        // The carried gap's texels are zero coverage.
        let cell = &arena[rasters[0].2.clone()];
        for row in 0..4 {
            for x in 3..3 + g {
                assert_eq!(cell[row * (6 + g) + x], 0);
            }
        }
        // Gap of `PARTIAL_GAP_MAX + 1`: two separate cells.
        let (emitted, _) = emit(&cov(&[(0, 3), (4 + g, 7 + g)], &[], 40, 4), &mut arena).unwrap();
        assert_eq!(emitted.cells.len(), 2);
        // A gap bounded by a full run (wider than `FULL_RUN_MIN`) is not
        // filled: the partial run ends before it and the full run spans.
        let (emitted, _) = emit(&cov(&[(0, 3)], &[(3 + g, 12 + g)], 40, 4), &mut arena).unwrap();
        assert_eq!(emitted.cells.len(), 1);
        assert_eq!(emitted.spans.len(), 1);
        // Adjacent partial edges must not consume the full interior run.
        let (emitted, rasters) =
            emit(&cov(&[(0, 3), (20, 23)], &[(3, 20)], 40, 4), &mut arena).unwrap();
        assert_eq!(emitted.spans.len(), 0);
        assert_eq!(emitted.cells.len(), 1);
        assert_eq!(emitted.cells[0].interior, 3 | (20 << 16));
        assert_eq!(rasters[0].0, 23);
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
    fn demoted_full_intervals_preserve_one_interior_cell() {
        // The short full interval becomes partial. All three partial
        // intervals must join before consuming the retained full interval.
        let runs = [(1, 0.5), (4, 1.0), (5, 0.5), (13, 1.0), (14, 0.5)];
        let mut intervals = Vec::new();
        let mut emit = PathEmit::default();
        let mut rasters = Vec::new();
        let mut arena = Vec::new();
        emit_strip_sparse(
            &runs,
            &[runs.len()],
            14,
            0.0,
            0.0,
            0,
            &mut intervals,
            &mut emit,
            &mut rasters,
            &mut arena,
            &mut 0,
        )
        .unwrap();
        assert_eq!(emit.spans, [] as [[f32; 4]; 0]);
        assert_eq!(emit.cells.len(), 1);
        assert_eq!(emit.cells[0].rect, [0.0, 0.0, 14.0, 1.0]);
        assert_eq!(emit.cells[0].interior, 5 | (13 << 16));
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
                    let coverage = [0.0, 1.0, 0.25][next() % 3];
                    row[x..end].fill(coverage);
                    runs.push((u32::try_from(end).unwrap(), coverage));
                    x = end;
                }
                ends.push(runs.len());
            }
            let mut dense = Vec::new();
            let mut sparse = Vec::new();
            strip_class(&strip, w, sh, &mut dense);
            strip_class_sparse(&runs, &ends, w, &mut sparse);
            let mut expanded = Vec::new();
            for &(end, class) in &sparse {
                expanded.resize(end as usize, class);
            }
            assert_eq!(dense, expanded);
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
