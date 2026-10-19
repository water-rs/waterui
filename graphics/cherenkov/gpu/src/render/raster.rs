//! The exact-area coverage rasterizer for glyph outlines and clipped paths.
//!
//! A port of the accumulation rasterizer from font-rs (`raster.rs`): every
//! flattened polygon edge accumulates a signed area into a `(w + 1) * h`
//! grid, then each row is prefix-summed and folded to coverage. The winding
//! accumulation makes overlapping contours cancel correctly.
//!
//! [`BandRaster`] carries the same deposits through two `STRIP_H`-row band
//! buffers — accumulation and coverage — walking the grid top to bottom, so
//! the working set scales with the grid's width instead of its area. Each
//! edge keeps a cursor — its origin and its next deposit row — so its
//! scanline math resumes across bands unchanged, and a deposit that wraps a
//! row forward rides its edge into the next band: every cell's contribution
//! order stays exactly as the dense walk wrote it.
//!
//! Both walks evaluate every row boundary from the edge's origin
//! ([`cherenkov::lowering::scanline`]), never by advancing an absolute x row by row, so a tall
//! edge does not accumulate rounding error down its length.

#[cfg(test)]
use std::mem::size_of;

use cherenkov::FillRule;
use cherenkov::lowering::scanline::{EdgeX, RowWalk, walk_rows};

use crate::render::path::STRIP_H;

/// A `w` × `h` signed-area accumulation buffer.
#[derive(Debug)]
pub struct Raster {
    w: usize,
    h: usize,
    a: Vec<f32>,
}

impl Raster {
    /// A raster of `w` × `h` cells.
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            a: vec![0.0; (w + 1) * h],
        }
    }

    /// Accumulates the signed area of the segment `p0 -> p1`.
    pub fn draw_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) {
        walk_rows(Line {
            raster: self,
            p: [x0, y0, x1, y1],
        });
    }

    /// [`Self::draw_line`] with row boundaries evaluated by `E`.
    #[inline(always)]
    #[expect(
        clippy::inline_always,
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "inlined into `walk_rows`' instantiation for `E`; a direct port of \
                  font-rs's scanline area accounting"
    )]
    fn accumulate<E: EdgeX>(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) {
        if (y0 - y1).abs() <= f32::EPSILON {
            return;
        }
        let (dir, x0, y0, x1, y1) = if y0 < y1 {
            (1.0, x0, y0, x1, y1)
        } else {
            (-1.0, x1, y1, x0, y0)
        };
        let line = E::new((x1 - x0) / (y1 - y0), x0);
        let mut x = if y0 < 0.0 { line.at(-y0) } else { x0 };
        let y_start = y0.max(0.0) as usize;
        let y_end = self.h.min((y1.ceil() as usize).min(self.h));
        let len = self.a.len();
        let a = &mut self.a;
        let mut put = a.as_mut_slice();
        for y in y_start..y_end {
            let row = y * (self.w + 1);
            let bottom = ((y + 1) as f32).min(y1);
            let dy = bottom - (y as f32).max(y0);
            let xnext = line.at(bottom - y0);
            deposit_row(row, dy * dir, x, xnext, len, &mut put);
            x = xnext;
        }
    }

    /// The accumulated coverage per cell, non-zero winding rule.
    pub fn coverage(&self) -> Vec<f32> {
        self.coverage_rule(FillRule::NonZero)
    }

    /// The accumulated coverage per cell under `rule`.
    ///
    /// Non-zero clamps the winding magnitude; even-odd folds it into a
    /// triangle wave of period 2 so odd windings cover and even ones don't.
    pub fn coverage_rule(&self, rule: FillRule) -> Vec<f32> {
        let mut out = vec![0.0; self.w * self.h];
        for y in 0..self.h {
            let row = y * (self.w + 1);
            let mut acc = 0.0;
            for x in 0..self.w {
                acc += self.a[row + x];
                out[y * self.w + x] = match rule {
                    FillRule::NonZero => acc.abs().min(1.0),
                    FillRule::EvenOdd => {
                        let a = acc.abs() % 2.0;
                        1.0 - (a - 1.0).abs()
                    }
                };
            }
        }
        out
    }
}

/// One [`Raster::draw_line`] call, run by [`walk_rows`].
struct Line<'a> {
    raster: &'a mut Raster,
    p: [f32; 4],
}

impl RowWalk for Line<'_> {
    type Output = ();

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "see `RowWalk::walk`")]
    fn walk<E: EdgeX>(self) {
        let [x0, y0, x1, y1] = self.p;
        self.raster.accumulate::<E>(x0, y0, x1, y1);
    }
}

/// The deposits one grid row `y` receives from an edge crossing
/// `[x, xnext)` with signed area `d`, delivered to `put` as flat
/// `(w + 1) * h` indices in the original write order. `row` is
/// `y * (w + 1)` and `grid_len` is `(w + 1) * h`.
#[expect(
    clippy::similar_names,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::suboptimal_flops,
    reason = "a direct port of font-rs's scanline area accounting"
)]
#[inline]
fn deposit_row(row: usize, d: f32, x: f32, xnext: f32, grid_len: usize, put: &mut impl Deposit) {
    let (xa, xb) = if x < xnext { (x, xnext) } else { (xnext, x) };
    let xa_floor = xa.floor();
    let xa_i = xa_floor as i32;
    let xb_ceil = xb.ceil();
    let xb_i = xb_ceil as i32;
    if xb_i <= xa_i + 1 {
        // The piece stays within one cell column.
        let xmf = x.midpoint(xnext) - xa_floor;
        let cell = row as isize + xa_i as isize;
        if cell >= 0 && (cell as usize) < grid_len {
            put.put(cell as usize, d - d * xmf);
            if (cell as usize) + 1 < grid_len {
                put.put(cell as usize + 1, d * xmf);
            }
        }
        return;
    }
    let s = (xb - xa).recip();
    let xa_f = xa - xa_floor;
    let a0 = 0.5 * s * (1.0 - xa_f) * (1.0 - xa_f);
    let xb_f = xb - xb_ceil + 1.0;
    let am = 0.5 * s * xb_f * xb_f;
    let cell = row as isize + xa_i as isize;
    if cell < 0 {
        return;
    }
    let cell = cell as usize;
    put.put(cell, d * a0);
    if xb_i == xa_i + 2 {
        put.put(cell + 1, d * (1.0 - a0 - am));
    } else {
        let a1 = s * (1.5 - xa_f);
        put.put(cell + 1, d * (a1 - a0));
        let mut xi = xa_i + 2;
        while xi < xb_i - 1 {
            let c = row as isize + xi as isize;
            if c >= 0 && (c as usize) < grid_len {
                put.put(c as usize, d * s);
            }
            xi += 1;
        }
        let a2 = a1 + (xb_i - xa_i - 3) as f32 * s;
        let last = row as isize + (xb_i - 1) as isize;
        if last >= 0 && (last as usize) < grid_len {
            put.put(last as usize, d * (1.0 - a2 - am));
        }
    }
    let xb_cell = row as isize + xb_i as isize;
    if xb_cell >= 0 && (xb_cell as usize) < grid_len {
        put.put(xb_cell as usize, d * am);
    }
}

/// The atlas's R8 texel for coverage `c`: the one quantization every
/// coverage upload uses, and the value strip classification judges a
/// column by, so cell layout depends only on what the atlas stores.
#[inline]
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the clamped, rounded value lies in 0..=255"
)]
pub fn texel(c: f32) -> u8 {
    (c.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The least coverage [`texel`] stores as nonzero: every smaller value
/// uploads as 0. Pinned to [`texel`] by a test at both sides of the step.
pub const TEXEL_NONZERO: f32 = f32::from_bits(0x3b00_8081);

/// The least coverage [`texel`] stores as `u8::MAX`. Pinned to [`texel`]
/// by a test at both sides of the step.
pub const TEXEL_FULL: f32 = f32::from_bits(0x3f7f_7f7f);

/// A signed-area destination. Whole-grid glyph rasters write directly;
/// band destinations additionally preserve deposits across their boundary.
trait Deposit {
    fn put(&mut self, index: usize, value: f32);
}

impl Deposit for &mut [f32] {
    #[inline]
    fn put(&mut self, index: usize, value: f32) {
        self[index] += value;
    }
}

/// Receives deposits in edge order without a branch per contribution.
trait DepositSink {
    fn add(&mut self, index: usize, local: usize, value: f32);
}

impl DepositSink for &mut [f32] {
    #[inline]
    fn add(&mut self, _index: usize, local: usize, value: f32) {
        self[local] += value;
    }
}

impl DepositSink for &mut Vec<(u32, f32)> {
    #[inline]
    fn add(&mut self, index: usize, _local: usize, value: f32) {
        self.push((u32::try_from(index).expect("raster index fits u32"), value));
    }
}

/// A band's deposit sink plus the window bookkeeping that lets a
/// deposit wrapping past the band's end ride `pending` into the edge's
/// next turn — the position the dense walk wrote it in — and drops a
/// backward wrap into the dead column of the band above.
struct Put<'a, S> {
    sink: S,
    lo: usize,
    len: usize,
    pending: &'a mut Option<(u32, f32)>,
}

impl<S: DepositSink> Deposit for Put<'_, S> {
    #[expect(
        clippy::inline_always,
        clippy::cast_possible_truncation,
        reason = "a call per deposit must not exist; grid indices stay under u32"
    )]
    #[inline(always)]
    fn put(&mut self, i: usize, v: f32) {
        if i.wrapping_sub(self.lo) < self.len {
            self.sink.add(i, i - self.lo, v);
        } else if i >= self.lo + self.len {
            debug_assert!(self.pending.is_none(), "a deposit outran the band walk");
            *self.pending = Some((i as u32, v));
        } else {
            // clip_x keeps the recurrence inside its row's slice, so a
            // backward wrap can only be a rounded x brushing under
            // zero — `i == lo - 1`, column `w` of the band above, the
            // dead column the dense walk writes and never reads.
            debug_assert!(i + 1 == self.lo, "deposit wrapped to a live cell: {i}");
        }
    }
}

/// One clipped edge's band-by-band cursor: its origin `(x0, y0)` and its
/// next deposit row `y`. Every row boundary, the top of the row a turn
/// resumes at included, is evaluated from the origin exactly as the dense
/// walk evaluates it, so resuming across a band boundary changes nothing.
#[derive(Debug, Clone)]
struct Edge {
    dir: f32,
    dxdy: f32,
    x0: f32,
    y0: f32,
    y1: f32,
    y: usize,
    y_end: usize,
    /// A deposit that wrapped past the current band. It is applied at the
    /// head of this edge's next turn, the position the dense walk wrote it
    /// in — after the edge's last deposits and before its next ones.
    pending: Option<(u32, f32)>,
}

impl Edge {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "same port as `Raster::accumulate`"
    )]
    fn new(x0: f32, y0: f32, x1: f32, y1: f32, h: usize) -> Option<Self> {
        if (y0 - y1).abs() <= f32::EPSILON {
            return None;
        }
        let (dir, x0, y0, x1, y1) = if y0 < y1 {
            (1.0, x0, y0, x1, y1)
        } else {
            (-1.0, x1, y1, x0, y0)
        };
        let dxdy = (x1 - x0) / (y1 - y0);
        let y = y0.max(0.0) as usize;
        let y_end = h.min((y1.ceil() as usize).min(h));
        (y < y_end).then_some(Self {
            dir,
            dxdy,
            x0,
            y0,
            y1,
            y,
            y_end,
            pending: None,
        })
    }

    /// Emits this edge's deposit cells in `[self.y, band_end)` through
    /// `out` as flat grid indices in `[lo, lo + len)` of the `stride`-wide
    /// grid, with row boundaries evaluated by `E`. A deposit outside the
    /// window is queued on the edge, or dropped when it wraps into the dead
    /// column of the band above.
    #[inline(always)]
    #[expect(
        clippy::inline_always,
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        clippy::cast_sign_loss,
        clippy::float_cmp,
        clippy::suboptimal_flops,
        reason = "inlined into `walk_rows`' instantiation for `E`; same port as \
                  `Raster::accumulate`; the origin's own row starts at x0 exactly"
    )]
    fn turn<E: EdgeX>(
        &mut self,
        band_end: usize,
        stride: usize,
        lo: usize,
        len: usize,
        grid_len: usize,
        mut sink: impl DepositSink,
    ) {
        if self.pending.is_some_and(|(i, _)| (i as usize) < lo + len) {
            let (i, v) = self.pending.take().expect("checked above");
            debug_assert!((i as usize) >= lo);
            sink.add(i as usize, (i as usize) - lo, v);
        }
        let end = self.y_end.min(band_end);
        // The cursor fields run as locals — the `pending` borrow inside
        // `put` would otherwise alias `self` and force a reload per row;
        // the row writes back once when the band's rows are done.
        let mut y = self.y;
        let (x0, y0, y1, dir, dxdy) = (self.x0, self.y0, self.y1, self.dir, self.dxdy);
        let line = E::new(dxdy, x0);
        // The x where row `y` starts, as the dense walk reached it: the
        // origin's own row starts at x0, a row above the raster's top at
        // the boundary 0, and every later row at the previous row's bottom.
        let top = (y as f32).max(y0);
        let mut x = if top == y0 { x0 } else { line.at(top - y0) };
        let mut pending = None;
        let mut put = Put {
            sink,
            lo,
            len,
            pending: &mut pending,
        };
        if dxdy == 0.0 {
            // A vertical edge: `xnext == x` every row, so `deposit_row`'s
            // single-cell branch is loop-invariant in x. The deposits
            // below are that branch verbatim (`x.midpoint(x) - floor(x)`,
            // `d - d * xmf`, `d * xmf`), with only `row` varying.
            let xa_floor = x.floor();
            let xa_i = xa_floor as i32;
            let xmf = x - xa_floor;
            while y < end {
                let row = y * stride;
                let d = (((y + 1) as f32).min(y1) - (y as f32).max(y0)) * dir;
                let cell = row as isize + xa_i as isize;
                if cell >= 0 && (cell as usize) < grid_len {
                    put.put(cell as usize, d - d * xmf);
                    if (cell as usize) + 1 < grid_len {
                        put.put(cell as usize + 1, d * xmf);
                    }
                }
                y += 1;
            }
        } else {
            while y < end {
                let row = y * stride;
                let bottom = ((y + 1) as f32).min(y1);
                let dy = bottom - (y as f32).max(y0);
                let xnext = line.at(bottom - y0);
                deposit_row(row, dy * dir, x, xnext, grid_len, &mut put);
                x = xnext;
                y += 1;
            }
        }
        self.y = y;
        if pending.is_some() {
            self.pending = pending;
        }
    }
}

/// A winding accumulation folded to coverage under `rule`.
fn fold(acc: f32, rule: FillRule) -> f32 {
    match rule {
        FillRule::NonZero => acc.abs().min(1.0),
        FillRule::EvenOdd => {
            let a = acc.abs() % 2.0;
            1.0 - (a - 1.0).abs()
        }
    }
}

/// A sealed band of coverage, in the representation its density chose.
#[derive(Debug)]
pub enum Band<'a> {
    /// Per-column coverage for `w` columns of `STRIP_H` rows.
    Dense(&'a [f32]),
    /// Per-row `(end_column, coverage)` constant-coverage runs: one
    /// flat buffer and each row's end offset, so the walk writes a
    /// band without a `Vec` per row.
    Sparse(&'a [(u32, f32)], &'a [usize]),
}

/// A band rasterizes sparsely while its deposit count stays under
/// `1 / SPARSE_MAX_EVENT_DENSITY` of the band's `w * rows` cells: past
/// it, sorting events costs more than the dense prefix over the band
/// buffer. The choice uses a conservative bound before collecting events,
/// so dense bands do not first pay for a discarded sparse pass.
const SPARSE_MAX_EVENT_DENSITY: usize = 4;

/// The banded coverage rasterizer: signed areas accumulate `STRIP_H` rows
/// at a time, each sealed band prefixes into coverage and streams out.
/// The two reused buffers bound the working set to the grid's width,
/// never its area.
#[derive(Debug)]
pub struct BandRaster {
    w: usize,
    h: usize,
    stride: usize,
    rule: FillRule,
    acc: Vec<f32>,
    cov: Vec<f32>,
    edges: Vec<Edge>,
    /// The sparse walk's band-local `(index, area)` deposits.
    events: Vec<(u32, f32)>,
    /// The sparse walk's per-row constant-coverage runs,
    /// `(end_column, coverage)`, in one buffer; `runs_off` holds each
    /// row's end offset for the band being walked.
    runs: Vec<(u32, f32)>,
    /// Row end offsets into `runs` for the current band.
    runs_off: [usize; STRIP_H],
    /// First edge starting in each band, followed through `next` in
    /// original edge order. Each edge appears exactly once in these links.
    starts: Vec<usize>,
    next: Vec<usize>,
    /// Active edges remain in original order so cell additions retain
    /// their association. `merged` is reused when a band introduces edges.
    active: Vec<usize>,
    merged: Vec<usize>,
}

impl BandRaster {
    /// An empty rasterizer; `reset` sizes it.
    pub const fn new() -> Self {
        Self {
            w: 0,
            h: 0,
            stride: 0,
            rule: FillRule::NonZero,
            acc: Vec::new(),
            cov: Vec::new(),
            edges: Vec::new(),
            events: Vec::new(),
            runs: Vec::new(),
            starts: Vec::new(),
            next: Vec::new(),
            active: Vec::new(),
            merged: Vec::new(),
            runs_off: [0; STRIP_H],
        }
    }

    /// Reuses the band buffers and edge list for a `w` × `h` grid under `rule`.
    pub fn reset(&mut self, w: usize, h: usize, rule: FillRule) {
        self.w = w;
        self.h = h;
        self.stride = w + 1;
        self.rule = rule;
        self.edges.clear();
        self.events.clear();
    }

    /// Schedule each edge only at its first band. A tall edge carries
    /// its cursor in the active list instead of allocating one membership
    /// node per band, so indexing costs O(edges + bands), not O(edges × bands).
    fn bucket(&mut self, n_bands: usize) {
        self.starts.clear();
        self.starts.resize(n_bands, usize::MAX);
        self.next.clear();
        self.next.resize(self.edges.len(), usize::MAX);
        self.active.clear();
        self.merged.clear();
        for (i, edge) in self.edges.iter().enumerate().rev() {
            let first = edge.y / STRIP_H;
            self.next[i] = self.starts[first];
            self.starts[first] = i;
        }
    }

    /// Remove finished cursors and merge newly starting edges in stable
    /// input order. A finished edge with a forward deposit remains active
    /// until that deposit has been applied at its original position.
    fn activate(&mut self, band: usize) {
        let edges = &self.edges;
        self.active.retain(|&i| {
            let edge = &edges[i];
            edge.y < edge.y_end || edge.pending.is_some()
        });
        let mut next = self.starts[band];
        if next == usize::MAX {
            return;
        }
        self.merged.clear();
        let mut cursor = 0;
        while cursor < self.active.len() || next != usize::MAX {
            if cursor < self.active.len() && self.active[cursor] < next {
                self.merged.push(self.active[cursor]);
                cursor += 1;
            } else {
                self.merged.push(next);
                next = self.next[next];
            }
        }
        std::mem::swap(&mut self.active, &mut self.merged);
    }

    /// Sizes the dense band buffers on first use; a path that never
    /// takes a dense band pays none of them.
    fn size_dense(&mut self) {
        self.acc.clear();
        self.acc.resize(self.stride * STRIP_H, 0.0);
        self.cov.clear();
        self.cov.resize(self.w * STRIP_H, 0.0);
    }

    /// Reserves the edge list for `n` more queued segments.
    pub fn reserve_edges(&mut self, n: usize) {
        self.edges.reserve(n);
    }

    /// Queues one clipped segment's cursor.
    pub fn draw(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) {
        if let Some(e) = Edge::new(x0, y0, x1, y1, self.h) {
            self.edges.push(e);
        }
    }

    /// Accumulates every queued edge band by band, pushing each sealed
    /// band's `rows * w` coverage values into `sink` in row order.
    pub fn finish<F: FnMut(&[f32])>(&mut self, sink: &mut F) {
        walk_rows(Finish { raster: self, sink });
    }

    /// [`Self::finish`] with row boundaries evaluated by `E`.
    #[inline(always)]
    #[expect(clippy::inline_always, reason = "see `RowWalk::walk`")]
    fn finish_with<E: EdgeX, F: FnMut(&[f32])>(&mut self, sink: &mut F) {
        self.size_dense();
        let (w, h, stride, rule) = (self.w, self.h, self.stride, self.rule);
        let grid_len = stride * h;
        let mut base = 0usize;
        while base < h {
            let rows = STRIP_H.min(h - base);
            let lo = base * stride;
            let len = rows * stride;
            let acc = self.acc.as_mut_slice();
            for e in &mut self.edges {
                e.turn::<E>(base + rows, stride, lo, len, grid_len, &mut *acc);
            }
            for r in 0..rows {
                let mut a = 0.0;
                for x in 0..w {
                    a += self.acc[r * stride + x];
                    self.cov[r * w + x] = fold(a, rule);
                }
                self.acc[r * stride..(r + 1) * stride].fill(0.0);
            }
            sink(&self.cov[..rows * w]);
            base += STRIP_H;
        }
        debug_assert!(self.edges.iter().all(|e| e.pending.is_none()));
    }

    /// The per-band choice: the band goes dense once its edges' deposit
    /// bound exceeds `1 / SPARSE_MAX_EVENT_DENSITY` of its cells; under
    /// it the band's deposits collect as events. `sink` receives the band
    /// and its base row.
    pub fn finish_choose<F: FnMut(Band<'_>, usize)>(&mut self, sink: &mut F) {
        walk_rows(FinishChoose { raster: self, sink });
    }

    /// [`Self::finish_choose`] with row boundaries evaluated by `E`.
    #[inline(always)]
    #[expect(
        clippy::inline_always,
        clippy::many_single_char_names,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "see `RowWalk::walk`; indices and deposit bounds are within the clipped \
                  raster grid"
    )]
    fn finish_choose_with<E: EdgeX, F: FnMut(Band<'_>, usize)>(&mut self, sink: &mut F) {
        let (w, h, stride, rule) = (self.w, self.h, self.stride, self.rule);
        let grid_len = stride * h;
        self.bucket(h.div_ceil(STRIP_H));
        let mut base = 0usize;
        while base < h {
            let rows = STRIP_H.min(h - base);
            let lo = base * stride;
            let len = rows * stride;
            self.activate(base / STRIP_H);
            // A conservative deposit bound chooses the representation
            // before writing: dense bands never allocate sparse events.
            let cap = rows * w / SPARSE_MAX_EVENT_DENSITY;
            let estimate: usize = self
                .active
                .iter()
                .map(|&i| {
                    let edge = &self.edges[i];
                    let count = edge.y_end.min(base + rows) - edge.y;
                    count * (edge.dxdy.abs().ceil() as usize + 3)
                        + usize::from(edge.pending.is_some())
                })
                .sum();
            let dense = estimate > cap;
            if dense {
                if self.acc.len() < stride * STRIP_H {
                    self.size_dense();
                }
                let acc = self.acc.as_mut_slice();
                for &i in &self.active {
                    self.edges[i].turn::<E>(base + rows, stride, lo, len, grid_len, &mut *acc);
                }
                for r in 0..rows {
                    let mut a = 0.0;
                    for x in 0..w {
                        a += self.acc[r * stride + x];
                        self.cov[r * w + x] = fold(a, rule);
                    }
                    self.acc[r * stride..(r + 1) * stride].fill(0.0);
                }
                sink(Band::Dense(&self.cov[..rows * w]), base);
            } else {
                // Sparse: the bound above keeps `events` under the cap;
                // collect the band's deposits and run-length them.
                self.events.clear();
                self.events.reserve(estimate);
                for &i in &self.active {
                    self.edges[i].turn::<E>(
                        base + rows,
                        stride,
                        lo,
                        len,
                        grid_len,
                        &mut self.events,
                    );
                }
                // The flat grid index orders events row-major; a stable
                // sort keeps each cell's deposits in write order.
                let events = &mut self.events;
                events.sort_by_key(|&(i, _)| i);
                self.runs.clear();
                let mut gi = 0usize;
                for r in 0..rows {
                    let row_lo = lo + r * stride;
                    let row_hi = row_lo + w;
                    let mut acc = 0.0;
                    let mut pos = 0u32;
                    let runs = &mut self.runs;
                    while gi < events.len() {
                        let i = events[gi].0 as usize;
                        if i >= row_hi {
                            break;
                        }
                        if i < row_lo {
                            // Column `w` of an earlier row — the dead
                            // column.
                            gi += 1;
                            continue;
                        }
                        let c = (i - row_lo) as u32;
                        if c > pos {
                            runs.push((c, fold(acc, rule)));
                        }
                        let mut s = 0.0;
                        while gi < events.len() && events[gi].0 == i as u32 {
                            s += events[gi].1;
                            gi += 1;
                        }
                        acc += s;
                        pos = c;
                    }
                    if pos < w as u32 {
                        runs.push((w as u32, fold(acc, rule)));
                    }
                    self.runs_off[r] = self.runs.len();
                }
                sink(Band::Sparse(&self.runs, &self.runs_off[..rows]), base);
            }
            base += STRIP_H;
        }
        debug_assert!(self.edges.iter().all(|e| e.pending.is_none()));
    }

    /// Heap bytes the reusable buffers hold between rasterizations.
    pub const fn heap_bytes(&self) -> u64 {
        (self.acc.capacity() * size_of::<f32>()
            + self.cov.capacity() * size_of::<f32>()
            + self.edges.capacity() * size_of::<Edge>()
            + self.events.capacity() * size_of::<(u32, f32)>()
            + self.runs.capacity() * size_of::<(u32, f32)>()
            + self.starts.capacity() * size_of::<usize>()
            + self.next.capacity() * size_of::<usize>()
            + self.active.capacity() * size_of::<usize>()
            + self.merged.capacity() * size_of::<usize>()) as u64
    }
}

impl Default for BandRaster {
    fn default() -> Self {
        Self::new()
    }
}

/// One [`BandRaster::finish`] call, run by [`walk_rows`].
struct Finish<'a, F> {
    raster: &'a mut BandRaster,
    sink: &'a mut F,
}

impl<F: FnMut(&[f32])> RowWalk for Finish<'_, F> {
    type Output = ();

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "see `RowWalk::walk`")]
    fn walk<E: EdgeX>(self) {
        self.raster.finish_with::<E, F>(self.sink);
    }
}

/// One [`BandRaster::finish_choose`] call, run by [`walk_rows`].
struct FinishChoose<'a, F> {
    raster: &'a mut BandRaster,
    sink: &'a mut F,
}

impl<F: FnMut(Band<'_>, usize)> RowWalk for FinishChoose<'_, F> {
    type Output = ();

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "see `RowWalk::walk`")]
    fn walk<E: EdgeX>(self) {
        self.raster.finish_choose_with::<E, F>(self.sink);
    }
}

#[cfg(test)]
mod tests {
    use cherenkov::lowering::scanline::{Fused, Widened};

    use super::*;

    /// Both row evaluations accumulate bit-identical areas, so coverage does
    /// not depend on whether the CPU has a fused multiply-add, and the band
    /// walk, resuming each edge from its origin at every band, deposits
    /// what the dense walk deposits.
    #[test]
    #[expect(
        clippy::cast_precision_loss,
        reason = "random bits shaped into coordinates"
    )]
    fn row_evaluations_accumulate_identical_areas() {
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        // Edges cross the top and bottom of the raster; columns stay inside
        // it, as the path rasterizer's callers guarantee.
        let mut coordinate = |low: f32, high: f32| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            ((state >> 40) as f32 / 16_777_216.0).mul_add(high - low, low)
        };
        let (w, h) = (257, 731);
        let (mut fused, mut widened) = (Raster::new(w, h), Raster::new(w, h));
        let mut banded = BandRaster::new();
        banded.reset(w, h, FillRule::NonZero);
        for _ in 0..20_000 {
            let (x0, y0) = (coordinate(0.0, 256.0), coordinate(-4.0, 735.0));
            let (x1, y1) = (coordinate(0.0, 256.0), coordinate(-4.0, 735.0));
            fused.accumulate::<Fused>(x0, y0, x1, y1);
            widened.accumulate::<Widened>(x0, y0, x1, y1);
            banded.draw(x0, y0, x1, y1);
        }
        assert!(
            fused
                .a
                .iter()
                .zip(&widened.a)
                .all(|(f, w)| f.to_bits() == w.to_bits()),
            "fused and widened row evaluations diverge"
        );
        let mut out = Vec::new();
        banded.finish(&mut |band: &[f32]| out.extend_from_slice(band));
        assert!(
            out.iter()
                .zip(&fused.coverage())
                .all(|(b, d)| b.to_bits() == d.to_bits()),
            "the band walk diverges from the dense walk"
        );
    }

    /// The issue's long edges (#2665) against the independent f64 area of
    /// the same polygon, through the dense walk and the banded walk the
    /// path rasterizer takes, in both directions and under both fill rules.
    #[test]
    fn tall_edges_agree_with_independent_pixel_area() {
        let cases = [
            (
                [(900.0_f32, 0.0_f32), (512.0, 900.0), (900.0, 900.0)],
                (598_u16, 699_u16),
            ),
            (
                [
                    (689.4518, 1.803_737_2),
                    (532.2187, 337.7357),
                    (668.394_17, 2.898_188_6),
                ],
                (534, 332),
            ),
        ];
        let size = 901;
        for (vertices, pixel) in cases {
            let (px, py) = (usize::from(pixel.0), usize::from(pixel.1));
            for reverse in [false, true] {
                let mut raster = Raster::new(size, size);
                // Translate only the independent f64 oracle to the target pixel.
                // The rasterizers must traverse the full height of each edge.
                let mut reference = cherenkov_oracle::coverage::Coverage::new(1, 1);
                let mut edges = Vec::new();
                for i in 0..vertices.len() {
                    let (mut a, mut b) = (vertices[i], vertices[(i + 1) % vertices.len()]);
                    if reverse {
                        std::mem::swap(&mut a, &mut b);
                    }
                    raster.draw_line(a.0, a.1, b.0, b.1);
                    edges.push((a, b));
                    reference.add_line(
                        f64::from(a.0) - f64::from(pixel.0),
                        f64::from(a.1) - f64::from(pixel.1),
                        f64::from(b.0) - f64::from(pixel.0),
                        f64::from(b.1) - f64::from(pixel.1),
                    );
                }
                let expected = reference.finish(cherenkov_scene::FillRule::NonZero)[0];
                for rule in [FillRule::NonZero, FillRule::EvenOdd] {
                    let dense = raster.coverage_rule(rule)[py * size + px];
                    let mut banded = BandRaster::new();
                    banded.reset(size, size, rule);
                    for &(a, b) in &edges {
                        banded.draw(a.0, a.1, b.0, b.1);
                    }
                    let mut band_value = None;
                    banded.finish_choose(&mut |band, base| {
                        if !(base..base + STRIP_H).contains(&py) {
                            return;
                        }
                        let row = py - base;
                        band_value = Some(match band {
                            Band::Dense(strip) => strip[row * size + px],
                            Band::Sparse(runs, ends) => {
                                let lo = if row == 0 { 0 } else { ends[row - 1] };
                                let runs = &runs[lo..ends[row]];
                                runs[runs.partition_point(|&(end, _)| end as usize <= px)].1
                            }
                        });
                    });
                    let banded = band_value.expect("the pixel's band was walked");
                    assert_eq!(
                        banded.to_bits(),
                        dense.to_bits(),
                        "{rule:?} reverse={reverse} ({px},{py}): band {banded} versus dense {dense}"
                    );
                    let actual = f64::from(dense);
                    assert!(
                        (actual - expected).abs() < 0.0002,
                        "{rule:?} reverse={reverse} ({px},{py}): {actual} versus {expected}"
                    );
                }
            }
        }
    }

    /// The classification bounds are exactly where the quantization steps:
    /// `texel` is monotonic, so the two sides of each step decide it.
    #[test]
    fn the_texel_bounds_are_the_quantization_steps() {
        assert_eq!(texel(TEXEL_NONZERO.next_down()), 0);
        assert_eq!(texel(TEXEL_NONZERO), 1);
        assert_eq!(texel(TEXEL_FULL.next_down()), u8::MAX - 1);
        assert_eq!(texel(TEXEL_FULL), u8::MAX);
    }

    #[test]
    fn a_half_pixel_square_has_exact_half_coverage() {
        // A 9x9 square at (0.5, 0.5) - (9.5, 9.5) in a 10x10 raster: every
        // boundary row/column is half covered.
        let mut raster = Raster::new(10, 10);
        let edges = [
            (0.5, 0.5, 9.5, 0.5),
            (9.5, 0.5, 9.5, 9.5),
            (9.5, 9.5, 0.5, 9.5),
            (0.5, 9.5, 0.5, 0.5),
        ];
        for (x0, y0, x1, y1) in edges {
            raster.draw_line(x0, y0, x1, y1);
        }
        let cov = raster.coverage();
        let at = |x: usize, y: usize| cov[y * 10 + x];
        for i in 1..9 {
            assert!((at(i, 0) - 0.5).abs() < 1e-6, "top edge {i}: {}", at(i, 0));
            assert!((at(i, 9) - 0.5).abs() < 1e-6, "bottom edge {i}");
            assert!((at(0, i) - 0.5).abs() < 1e-6, "left edge {i}");
            assert!((at(9, i) - 0.5).abs() < 1e-6, "right edge {i}");
        }
        for y in 1..9 {
            for x in 1..9 {
                assert!((at(x, y) - 1.0).abs() < 1e-6, "interior ({x}, {y})");
            }
        }
    }

    #[test]
    fn a_triangles_total_coverage_matches_its_area() {
        // Right triangle (1,1)-(9,1)-(9,9): area = 32 pixels.
        let mut raster = Raster::new(16, 16);
        raster.draw_line(1.0, 1.0, 9.0, 1.0);
        raster.draw_line(9.0, 1.0, 9.0, 9.0);
        raster.draw_line(9.0, 9.0, 1.0, 1.0);
        let total: f32 = raster.coverage().iter().sum();
        assert!((total - 32.0).abs() < 1e-3, "triangle area: {total}");
    }

    #[test]
    #[expect(
        clippy::unreadable_literal,
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "fixed LCG constants and index-to-coordinate test math"
    )]
    fn a_band_walk_deposits_what_the_dense_walk_deposits() {
        // Deterministic pseudo-random edges inside a grid taller than a
        // band: the banded walk and the dense walk produce the same
        // coverage, under both fill rules.
        let (w, h) = (37usize, 13usize);
        let mut seed = 0x9e3779b97f4a7c15u64;
        let mut rand = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((seed >> 33) as f32) / (1u32 << 31) as f32
        };
        let edges: Vec<(f32, f32, f32, f32)> = (0..40)
            .map(|_| {
                (
                    rand() * w as f32,
                    rand() * h as f32,
                    rand() * w as f32,
                    rand() * h as f32,
                )
            })
            .collect();
        for rule in [FillRule::NonZero, FillRule::EvenOdd] {
            let mut dense = Raster::new(w, h);
            let mut banded = BandRaster::new();
            let mut sparse = BandRaster::new();
            banded.reset(w, h, rule);
            sparse.reset(w, h, rule);
            for &(x0, y0, x1, y1) in &edges {
                dense.draw_line(x0, y0, x1, y1);
                banded.draw(x0, y0, x1, y1);
                sparse.draw(x0, y0, x1, y1);
            }
            let mut out = Vec::new();
            banded.finish(&mut |band: &[f32]| out.extend_from_slice(band));
            assert_eq!(out, dense.coverage_rule(rule), "rule {rule:?}");
            let mut out = Vec::new();
            sparse.finish_choose(&mut |band, base| {
                assert_eq!(out.len(), base * w, "band order at base {base}");
                match band {
                    Band::Dense(strip) => out.extend_from_slice(strip),
                    Band::Sparse(runs, ends) => {
                        let mut lo = 0;
                        for &hi in ends {
                            let row = &runs[lo..hi];
                            lo = hi;
                            for c in 0..w as u32 {
                                out.push(row[row.partition_point(|&(end, _)| end <= c)].1);
                            }
                        }
                    }
                }
            });
            assert_eq!(out, dense.coverage_rule(rule), "choose rule {rule:?}");
        }
    }

    #[test]
    #[expect(clippy::cast_precision_loss)]
    fn reused_buffers_keep_dense_and_sparse_bands_identical() {
        let mut banded = BandRaster::new();
        for (w, h, count) in [(1024, 17, 2), (37, 13, 32), (2048, 9, 2), (19, 7, 24)] {
            for rule in [FillRule::NonZero, FillRule::EvenOdd] {
                let mut dense = Raster::new(w, h);
                banded.reset(w, h, rule);
                for i in 0..count {
                    let x = (i + 1) as f32 * w as f32 / (count + 1) as f32;
                    let edge = (x, 0.25, (x + 0.75).min(w as f32), h as f32 - 0.25);
                    dense.draw_line(edge.0, edge.1, edge.2, edge.3);
                    banded.draw(edge.0, edge.1, edge.2, edge.3);
                }
                let expected = dense.coverage_rule(rule);
                banded.finish_choose(&mut |band, base| match band {
                    Band::Dense(coverage) => {
                        for (actual, expected) in coverage.iter().zip(&expected[base * w..]) {
                            assert_eq!(actual.to_bits(), expected.to_bits());
                        }
                    }
                    Band::Sparse(runs, ends) => {
                        let mut start = 0;
                        for (row, &end) in ends.iter().enumerate() {
                            let mut column = 0;
                            for &(next, actual) in &runs[start..end] {
                                for expected in &expected
                                    [(base + row) * w + column..(base + row) * w + next as usize]
                                {
                                    assert_eq!(actual.to_bits(), expected.to_bits());
                                }
                                column = next as usize;
                            }
                            start = end;
                        }
                    }
                });
            }
        }
    }

    #[test]
    fn a_sparse_path_never_materializes_the_dense_band() {
        // A thin diagonal through a large grid: every band stays under
        // the event-density threshold, so the heap stays at the event
        // buffers — far below the dense band pair, let alone the
        // (w+1) x h accumulation the dense raster would have held.
        let (w, h) = (1992usize, 1992usize);
        let mut banded = BandRaster::new();
        banded.reset(w, h, FillRule::NonZero);
        banded.draw(28.0, 28.0, 1964.0, 1964.0);
        banded.finish_choose(&mut |_, _| {});
        let heap = banded.heap_bytes();
        assert!(
            heap < (w * STRIP_H * size_of::<f32>()) as u64,
            "heap {heap} vs dense band {} ",
            w * STRIP_H * size_of::<f32>()
        );
    }
}
