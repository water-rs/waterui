//! Banded rendering from sparse, winding-resolved coverage runs.

use super::coverage::{Compiler, Run, Source};
use arrayvec::ArrayVec;
use half::slice::HalfFloatSliceExt;
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::num::NonZeroUsize;
use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};

use cherenkov::FillRule;
use cherenkov_simd::math::erf4;

use crate::render::bins::{FrameIndex, FrameMeter, Selection, SelectionBuffer};
use crate::render::lower::{
    BoxClip, CaptureItem, ClipMask, ClipRef, FrameFilter, IRect, Item, SampleEffect, SdfEffect,
    SdfKind, UnionSample,
};
use crate::render::paint::PaintData;
use crate::render::shadow;
use crate::render::tiles::EdgeTiles;
use crate::{Band as BandOut, BandPixels};
use cherenkov::OffscreenFormat;
use cherenkov_simd::math::{exp4, ln4};
use fearless_simd::{Select, Simd, SimdBase, SimdInterleaved, f32x4};
use filtrate_core::space::{
    P3_TO_SRGB, SRGB_ENCODED_BREAK, SRGB_GAMMA, SRGB_INVERSE_GAMMA, SRGB_LINEAR_BREAK, SRGB_OFFSET,
    SRGB_SCALE, SRGB_SLOPE, SRGB_TO_P3,
};
use filtrate_core::{CpuFilterError, CpuImage, WorkingSpace};

/// Rows per rasterization band.
pub const BAND_H: usize = 16;

/// Texel rows a reduced capture keeps past the rows its sampled device
/// rows' bilinear taps reach, on each side: a tap that float rounding
/// moves across a texel boundary still lands on a kept row.
pub const SAMPLE_MARGIN: usize = 1;

/// The run length one span paint evaluation covers: a longer run is
/// evaluated and composited in `SPAN_CHUNK`-pixel pieces against one
/// stack-resident buffer.
const SPAN_CHUNK: usize = 64;

/// One job's reusable raster and filter buffers.
///
/// Created once per raster task, at most one task per worker, for the
/// duration of a pass. Each task dynamically claims whole bands;
/// buffers are single-owner, never locked, and nothing is retained after the
/// pass.
///
/// The transient pixel bound is `workers × (band + Σ filter windows at the
/// deepest nesting)`, where each filter window is `w × (bh + 2·apron)`.
pub struct Scratch<'a> {
    /// Sparse compiler storage, reused across bands.
    coverage: Compiler,
    /// Isolation-stack colour buffers currently in use.
    stack: Vec<Plane>,
    buffers: Buffers,
    /// This band's captured backdrops, by group id.
    captures: Captures,
    /// The frame's transient-memory meter; charged query scratch is
    /// released when this scratch is dropped — mid-call on a recycled
    /// worker or at the call's end.
    meter: Option<&'a FrameMeter>,
}

struct Buffers {
    free: Vec<Vec<[f32; 4]>>,
    coverage: Vec<Compiler>,
    clips: Vec<ClipScratch>,
    clip_charge: u64,
    /// Live colour-buffer bytes and their peak for the current band.
    meter: Meter,
    /// Expanded-window query scratch: each `run` depth's item selection.
    /// Slots keep their capacity across sibling scopes and bands.
    sel_items: Vec<SelectionBuffer>,
    /// The capacity each charged slot has been accounted for, in
    /// cells — a selection's capacity only grows, so the delta is the
    /// heap growth. Index `sel_items.len()` is the outer Vec's charge.
    sel_charge: Vec<usize>,
    /// `sel_items`'s own allocation, charged at capacity.
    sel_items_charge: usize,
}

struct BandScratch<'a> {
    stack: &'a mut Vec<Plane>,
    buffers: &'a mut Buffers,
    captures: &'a mut Captures,
}

/// Live pixel-buffer accounting: pixels held across a band's run.
#[derive(Default)]
struct Meter {
    live: usize,
    /// `live` at reset: the caller's own band buffer, not capture cost.
    base: usize,
    peak: usize,
    /// Whether a backdrop capture ran in the band being metered.
    captured: bool,
}

/// One isolation level's pixels: premultiplied in `space`.
struct Plane {
    /// The level's pixels.
    buf: Vec<[f32; 4]>,
    /// The level's storage space.
    space: cherenkov::BlendSpace,
}

/// The rows one group's captured backdrop holds this band, on the
/// capture grid: texel `i` covers the device interval `[i/s, (i+1)/s)`.
struct Capture {
    /// `w × rows` premultiplied texels starting at texel row `y0`.
    buf: Vec<[f32; 4]>,
    /// The capture scale `s`.
    scale: cherenkov::CaptureScale,
    /// First kept texel row.
    y0: usize,
    /// Kept texel row count.
    rows: usize,
    /// First texel column (the capture region's `x0`).
    x0: usize,
    /// Row stride (the capture region's width in texels).
    w: usize,
    /// The capture region's first texel row (`region.y0`) on the
    /// device-anchored capture grid — a multiple of `2^(n−1)` on an
    /// `n`-level capture. `y0` is an absolute grid row like it; each
    /// deeper level's `CaptureLevel::y0` counts from this origin's
    /// level-k row `ry0 / 2^k`.
    ry0: usize,
    /// The pyramid's deeper levels: `levels[k − 1]` holds level `k`'s
    /// kept texel rows, `k` in `1..n`; empty on a one-level capture.
    levels: Vec<CaptureLevel>,
    /// The device rows `[y0, y1)` this band's samples read.
    device_rows: (usize, usize),
    /// The device columns `[x0, x1)` the region's texels cover.
    device_cols: (usize, usize),
    /// The space the captured rows are stored in.
    space: cherenkov::BlendSpace,
}

/// A pyramid level's kept rows: level `k`'s texels, `w` columns wide
/// (the level's spec extent), starting at level-k row `y0`.
struct CaptureLevel {
    /// `w × rows` premultiplied texels starting at level-k row `y0`.
    buf: Vec<[f32; 4]>,
    /// First kept level-k row, in level-k texels counting from the
    /// region's level-k origin.
    y0: usize,
    /// Kept level-k row count.
    rows: usize,
    /// The level's spec width in texels (`⌈w / 2^k⌉`).
    w: usize,
}

/// Where one capture lands for a surface band (see [`capture_rows`]).
#[derive(Clone, Copy, Debug)]
struct CaptureRows {
    /// The device rows the band's samples read: the union's rows within
    /// the capture's reach of the band.
    device: (usize, usize),
    /// The capture texel rows kept for them.
    kept: (usize, usize),
    /// The device rows the run window must hold for the chain's window
    /// around the kept texels, before clamping to the region.
    window: (usize, usize),
    /// Levels 1..n's kept texel-row ranges, the first `n − 1` entries
    /// used; each counts from that level's grid origin.
    levels: [(usize, usize); cherenkov::CaptureLevels::MAX as usize],
}

/// The rows capture `item` covers when its members' samples are read
/// over the device rows `need` of a surface `w × h`, `None` when those
/// rows sample none of it.
///
/// `need` is the surface band, widened for a top-level capture by the
/// window of every later top-level capture this band runs that reads
/// the rows this group's members draw ([`top_captures`]). The device
/// rows are the union's rows within the capture's reach of `need`. The
/// item list's last capture, when its members displace their reads,
/// keeps only the rows its members' taps reach ([`sample_rows`]); any
/// other capture keeps every device row. A 1:1 capture keeps
/// exactly the sampled device rows, and its window is them `± apron`. A reduced capture keeps the texel rows the
/// sampled rows' bilinear taps at `(y + ½)·s − ½` reach, plus
/// [`SAMPLE_MARGIN`], within the region; its window is the device rows
/// under those texels `± apron` texels.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "rows are bounded by the surface height and clamped before the conversions"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn capture_rows(
    level: fearless_simd::Level,
    item: &CaptureItem,
    items: &[Item],
    need: (usize, usize),
    (width, h): (usize, usize),
    meter: Option<&FrameMeter>,
) -> Option<CaptureRows> {
    let (mut k0, mut k1) = kept_rows(&item.union, item.reach, need, h);
    let device = (k0, k1);
    if item.displaced
        && items.iter().rev().find_map(|item| match item {
            Item::Capture(capture) => Some(capture.group),
            _ => None,
        }) == Some(item.group)
    {
        let sampled = sample_rows_at(
            level,
            items,
            0..items.len(),
            item.group,
            need,
            (width, h),
            meter,
        );
        k0 = k0.max(sampled.0);
        k1 = k1.min(sampled.1);
    }
    if k0 >= k1 {
        return None;
    }
    if item.scale.is_full() && item.levels == 1 {
        return Some(CaptureRows {
            device,
            kept: (k0, k1),
            window: (k0.saturating_sub(item.apron), k1.saturating_add(item.apron)),
            levels: [(0, 0); cherenkov::CaptureLevels::MAX as usize],
        });
    }
    let s = f64::from(item.scale.get());
    let (ty0, ty1) = (
        usize::try_from(item.region.y0).unwrap_or(0),
        usize::try_from(item.region.y1).unwrap_or(0),
    );
    let first = (k0 as f64 + 0.5).mul_add(s, -0.5).floor().max(0.0) as usize;
    let last = (k1 as f64 - 0.5).mul_add(s, -0.5).floor().max(0.0) as usize + 2;
    let mut kept = (
        first.saturating_sub(SAMPLE_MARGIN).max(ty0),
        (last + SAMPLE_MARGIN).min(ty1),
    );
    let mut levels = [(0usize, 0usize); cherenkov::CaptureLevels::MAX as usize];
    if item.levels > 1 {
        // Level `k`'s tap range: the sampled device rows read level-k
        // texels `floor((d ± ½)·s/2^k − ½)` through `+1`, clamped to the
        // level's spec rows — the region's `y0` is 2^k-aligned, so the
        // level-k grid runs `ty0 / 2^k .. ⌈ty1 / 2^k⌉` in absolute
        // level-k rows.
        for (k, slot) in levels.iter_mut().enumerate().take(item.levels as usize - 1) {
            let k = k as u64 + 1;
            let d = (1u64 << k) as f64;
            let (gy0, gy1) = (ty0 >> k, ty1.div_ceil(1 << k));
            let first = ((k0 as f64 + 0.5) * s / d - 0.5).floor().max(0.0) as usize;
            let last = ((k1 as f64 - 0.5) * s / d - 0.5).floor().max(0.0) as usize + 2;
            *slot = (first.clamp(gy0, gy1), last.min(gy1));
        }
        // Level-k texel `r` reads level `k−1` texels `{2r, 2r+1}` — each
        // level's kept rows must cover the deeper level's parents,
        // folding down into the level-0 window the capture keeps.
        for k in (1..item.levels as usize).rev() {
            let (r0, r1) = levels[k - 1];
            if r0 >= r1 {
                continue;
            }
            let (lo, hi) = (2 * r0, (2 * r1).min(ty1.div_ceil(1 << (k - 1))));
            if k == 1 {
                kept = (kept.0.min(lo), kept.1.max(hi));
            } else {
                levels[k - 2].0 = levels[k - 2].0.min(lo);
                levels[k - 2].1 = levels[k - 2].1.max(hi);
            }
        }
    }
    if kept.0 >= kept.1 {
        return None;
    }
    let (w0, w1) = (
        kept.0.saturating_sub(item.apron),
        kept.1.saturating_add(item.apron),
    );
    Some(CaptureRows {
        device,
        kept,
        window: (
            (w0 as f64 / s).floor() as usize,
            (w1 as f64 / s).ceil() as usize,
        ),
        levels,
    })
}

/// Device rows touched by a group's actual member samples. The recursive
/// walk carries each enclosing filter's expanded draw window. Displaced
/// reads use the same field and coordinate evaluation as compositing;
/// plain reads need only their draw-row interval.
/// `sample_rows` exactly once per dispatched CPU level, like [`run_at`].
#[inline(never)]
fn sample_rows_at(
    level: fearless_simd::Level,
    items: &[Item],
    range: Range<usize>,
    group: u64,
    band: (usize, usize),
    (w, h): (usize, usize),
    meter: Option<&FrameMeter>,
) -> (usize, usize) {
    cherenkov_simd::dispatch!(level, _simd => {
        sample_rows(level, items, range, group, band, (w, h), meter)
    })
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "sample coordinates are finite and clamped to surface rows"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn sample_rows(
    level: fearless_simd::Level,
    items: &[Item],
    range: Range<usize>,
    group: u64,
    band: (usize, usize),
    (w, h): (usize, usize),
    meter: Option<&FrameMeter>,
) -> (usize, usize) {
    let mut rows = (h, 0);
    let mut clips = ClipScratch::default();
    let mut compiler = Compiler::default();
    let mut charged = 0;
    let mut i = range.start;
    while i < range.end {
        match &items[i] {
            Item::PushFilter { end, apron, .. } => {
                let end = *end as usize;
                let nested = sample_rows_at(
                    level,
                    items,
                    i + 1..end,
                    group,
                    (band.0.saturating_sub(*apron), (band.1 + apron).min(h)),
                    (w, h),
                    meter,
                );
                rows = (rows.0.min(nested.0), rows.1.max(nested.1));
                i = end;
            }
            Item::Sample {
                group: sampled,
                bounds,
                clip,
                effect,
                union,
                ..
            } if *sampled == group => {
                let first = (bounds.y0.max(0) as usize).max(band.0);
                let last = (bounds.y1.max(0) as usize).min(band.1);
                if first < last {
                    if let SampleEffect::Sdf(SdfEffect {
                        kind: SdfKind::Refraction { depth, strength },
                        ..
                    }) = effect
                    {
                        let x0 = (bounds.x0.max(0) as usize).min(w);
                        let x1 = (bounds.x1.max(0) as usize).min(w);
                        let clip = clips.prepare(
                            clip.as_ref(),
                            first..last,
                            w,
                            &mut compiler,
                            meter.map(|m| (&mut charged, m)),
                        );
                        for py in first..last {
                            for px in x0..x1 {
                                if clip_cov(clip, w, px, py) <= 0.0 {
                                    continue;
                                }
                                let (field, coverage) =
                                    sample_field(effect, union.as_ref(), px, py);
                                if coverage <= 0.0 {
                                    continue;
                                }
                                let (_, qy) = refracted(field, *depth, *strength, px, py);
                                let first = ((qy - 0.5).floor().max(0.0) as usize).min(h - 1);
                                rows.0 = rows.0.min(first);
                                rows.1 = rows.1.max(first.saturating_add(2).min(h));
                            }
                        }
                    } else {
                        rows = (rows.0.min(first), rows.1.max(last));
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    if let Some(meter) = meter {
        meter.query_live.fetch_sub(charged, Ordering::Relaxed);
        meter.clip_live.fetch_sub(charged, Ordering::Relaxed);
    }
    rows
}

/// This band's captures by backdrop group id.
type Captures = FxHashMap<u64, Capture>;

/// The rows of each top-level capture item this band serves, by item
/// index, in item order (see [`top_captures`]).
type TopCaptures = Vec<(usize, Option<CaptureRows>)>;

/// Per-band state shared across the recursive `run` calls.
struct FrameCtx<'a> {
    /// The dispatched CPU level; each recursive `run` re-dispatches under it.
    level: fearless_simd::Level,
    /// The surface band rows `[y0, y1)` this run derives from.
    surface: (usize, usize),
    /// The surface height.
    h: usize,
    /// The band's captured backdrops.
    captures: &'a mut Captures,
    /// The band's top-level capture rows; captures inside filter scopes
    /// are planned against the surface band by the scope's own run.
    top: &'a [(usize, Option<CaptureRows>)],
    /// The frame's transient-memory meter for expanded-window query
    /// scratch.
    meter: Option<&'a FrameMeter>,
}

/// Charges `bytes` of freshly grown query-scratch capacity to the
/// frame's live total and raises its peak. Called only on allocation
/// transitions — a pooled selection keeps its charge until its
/// scratch is dropped at the call's end.
fn sel_alloc(bytes: u64, meter: Option<&FrameMeter>) {
    if let Some(m) = meter {
        let live = m
            .query_live
            .fetch_add(bytes, std::sync::atomic::Ordering::Relaxed)
            + bytes;
        m.query_peak
            .fetch_max(live, std::sync::atomic::Ordering::Relaxed);
        m.peak
            .fetch_max(m.index_bytes + live, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Meter {
    /// Resets the per-band peak around the caller's live buffers (the
    /// output band buffer is held across `shade` on streamed targets).
    const fn reset(&mut self) {
        self.base = self.live;
        self.peak = self.live;
        self.captured = false;
    }

    const fn take(&mut self, len: usize) {
        self.live += len;
        self.peak = if self.peak < self.live {
            self.live
        } else {
            self.peak
        };
    }

    const fn give(&mut self, len: usize) {
        self.live -= len;
    }

    /// The capture-attributable peak: live pixels above the caller's
    /// baseline at the band's fullest point.
    const fn delta_peak(&self) -> usize {
        self.peak - self.base
    }
}

impl<'a> Scratch<'a> {
    /// An empty working set; buffers grow to band size on first use.
    pub fn new(meter: Option<&'a FrameMeter>, width: usize, edges: Option<usize>) -> Self {
        let mut coverage = Compiler::default();
        if let Some(edges) = edges {
            coverage.reserve(width, edges, BAND_H);
        }
        Self {
            coverage,
            stack: Vec::new(),
            buffers: Buffers {
                free: Vec::new(),
                coverage: Vec::new(),
                clips: Vec::new(),
                clip_charge: 0,
                meter: Meter::default(),
                sel_items: Vec::new(),
                sel_charge: Vec::new(),
                sel_items_charge: 0,
            },
            captures: Captures::default(),
            meter,
        }
    }
}

impl Drop for Scratch<'_> {
    /// Releases the charged query-scratch capacity: the pooled
    /// selections plus all retained bookkeeping heap (the slot Vec and
    /// the charge table itself). Bytes leave `query_live` here, at the
    /// buffers' actual destruction — including mid-call worker
    /// recycling — not at the call's end.
    fn drop(&mut self) {
        if let Some(m) = self.meter {
            let bytes = (self.buffers.sel_items_charge
                + self.buffers.sel_charge.iter().sum::<usize>() * size_of::<usize>())
                as u64
                + self.buffers.clip_charge;
            m.query_live
                .fetch_sub(bytes, std::sync::atomic::Ordering::Relaxed);
            m.clip_live
                .fetch_sub(self.buffers.clip_charge, Ordering::Relaxed);
        }
    }
}

impl Buffers {
    /// A zeroed colour buffer of `len` pixels, reused where possible.
    fn take_color(&mut self, len: usize) -> Vec<[f32; 4]> {
        let mut buf = self.free.pop().unwrap_or_default();
        buf.clear();
        buf.resize(len, [0.0; 4]);
        self.meter.take(len);
        buf
    }

    /// Returns a colour buffer to the freelist.
    fn give_color(&mut self, buf: Vec<[f32; 4]>) {
        self.meter.give(buf.len());
        self.free.push(buf);
    }

    fn take_coverage(&mut self, width: usize, edges: usize, height: usize) -> Compiler {
        let mut compiler = self.coverage.pop().unwrap_or_default();
        compiler.reserve(width, edges, height);
        compiler
    }

    fn give_coverage(&mut self, buf: Compiler) {
        self.coverage.push(buf);
    }

    /// The `depth` slot's item-selection buffer, cleared for reuse.
    /// Charges growth of the retained bookkeeping heap — the slot Vec
    /// and the charge table — to `meter`.
    fn take_sel_items(&mut self, depth: usize, meter: Option<&FrameMeter>) -> SelectionBuffer {
        if self.sel_items.len() <= depth {
            self.sel_items
                .resize_with(depth + 1, SelectionBuffer::default);
            self.sel_charge.resize(depth + 1, 0);
            let book = self.sel_items.capacity() * size_of::<SelectionBuffer>()
                + self.sel_charge.capacity() * size_of::<usize>();
            if book > self.sel_items_charge {
                sel_alloc((book - self.sel_items_charge) as u64, meter);
                self.sel_items_charge = book;
            }
        }
        let mut buf = std::mem::take(&mut self.sel_items[depth]);
        buf.clear();
        buf
    }

    /// Charges `buf`'s capacity growth on the `depth` slot to `meter`.
    /// Called right after the query fills `buf`, while the selection is
    /// live — before the recursive run or draw consumes it.
    fn charge_sel_items(
        &mut self,
        depth: usize,
        buf: &SelectionBuffer,
        meter: Option<&FrameMeter>,
    ) {
        let charged = &mut self.sel_charge[depth];
        if buf.capacity() > *charged {
            sel_alloc(
                ((buf.capacity() - *charged) * size_of::<usize>()) as u64,
                meter,
            );
            *charged = buf.capacity();
        }
    }

    /// Returns a selection to its slot: restores ownership only; the
    /// charge was taken when the buffer filled.
    fn give_sel_items(&mut self, depth: usize, buf: SelectionBuffer) {
        self.sel_items[depth] = buf;
    }
}

/// A directed edge in device space.
#[derive(Clone, Copy, Debug)]
pub struct Edge {
    /// Start point.
    pub x0: f32,
    /// Start point.
    pub y0: f32,
    /// End point.
    pub x1: f32,
    /// End point.
    pub y1: f32,
}

impl From<Edge> for (f32, f32, f32, f32) {
    fn from(e: Edge) -> Self {
        (e.x0, e.y0, e.x1, e.y1)
    }
}

/// `exp(-x²/2σ²) / (σ√2π)`.
#[expect(clippy::excessive_precision, reason = "sqrt(2π) to f32 accuracy")]
fn gaussian(x: f32, sigma: f32) -> f32 {
    (-(x * x) / (2.0 * sigma * sigma)).exp() / (2.506_628_274_6 * sigma)
}

/// Horizontal inset of a circular corner of radius `r` at distance `dy`
/// past the start of the corner (`dy <= 0` is the straight edge).
#[expect(clippy::suboptimal_flops, reason = "the reference formula verbatim")]
fn corner_inset(r: f32, dy: f32) -> f32 {
    if dy <= 0.0 || r <= 0.0 {
        return 0.0;
    }
    let dd = dy.min(r);
    r - (r * r - dd * dd).max(0.0).sqrt()
}

/// The shadow's premultiplied source at coverage `cov` under clip `cc`.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn shadow_src(color: &[f32; 4], cov: f32, cc: f32) -> [f32; 4] {
    let cov = cov.clamp(0.0, 1.0);
    [
        color[0] * cov * cc,
        color[1] * cov * cc,
        color[2] * cov * cc,
        color[3] * cov * cc,
    ]
}

/// Composites a run of shadow rows (rows with the same x intervals): the
/// edge columns, whose x integrals do not saturate, evaluate each
/// interval once per four pixels for every row of the run; the interior
/// columns between them take each row's weight sum. Writes are gated by
/// `clip_mask`.
#[expect(
    clippy::too_many_arguments,
    reason = "the item's shared context travels together"
)]
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "pixel indices and band offsets are far below 2^24"
)]
#[expect(
    clippy::option_if_let_else,
    reason = "a map_or closure inside the vectorize body is compiled without \
              the dispatched level's target features"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn shadow_run<S: Simd>(
    simd: S,
    dst: &mut [[f32; 4]],
    w: usize,
    y0: usize,
    cx: f32,
    inv_sqrt2_sigma: f32,
    margin: f32,
    clip_mask: Option<ClipCoverage<'_>>,
    stored: &[f32; 4],
    (cx_lo, cx_hi): (usize, usize),
    run: &shadow::Run,
    integrals: &mut shadow::Integrals<S>,
) {
    // Interior columns: `px >= xlm + margin` saturates every
    // `erf((xl-px)*is)` at -1 and `px <= xrm - margin` saturates every
    // `erf((xr-px)*is)` at +1, so `cov = s` for all of them.
    let x_in_lo = usize::try_from((cx + run.xlm + margin - 0.5).ceil() as i32)
        .unwrap_or(0)
        .clamp(cx_lo, cx_hi);
    let x_in_hi = usize::try_from((cx + run.xrm - margin + 0.5).floor() as i32 + 1)
        .unwrap_or(0)
        .clamp(x_in_lo, cx_hi);
    for (from, to) in [(cx_lo, x_in_lo), (x_in_hi, cx_hi)] {
        let mut x = from;
        while x < to {
            // One pixel centre per lane, each `(x as f32 + 0.5) - cx`; a
            // 1–3-lane tail pads with the first lane's centre and its
            // outputs are never read.
            let lanes = (to - x).min(4);
            let mut centres = [(x as f32 + 0.5) - cx; 4];
            for (j, c) in centres.iter_mut().enumerate().take(lanes) {
                *c = (x + j) as f32 + 0.5 - cx;
            }
            // The same saturation per group: columns at least `margin`
            // inside every right (left) edge skip the right (left) `erf`.
            let sides = match (
                centres[0] < run.xlm + margin,
                centres[lanes - 1] > run.xrm - margin,
            ) {
                (true, false) => shadow::Sides::Left,
                (false, true) => shadow::Sides::Right,
                _ => shadow::Sides::Both,
            };
            run.integrals4(
                simd,
                f32x4::load_array(simd, centres),
                inv_sqrt2_sigma,
                sides,
                integrals,
            );
            for (i, (y, _)) in run.rows().enumerate() {
                for (j, cov) in run
                    .coverage4(simd, i, integrals)
                    .to_array()
                    .into_iter()
                    .enumerate()
                    .take(lanes)
                {
                    if cov <= 0.0 {
                        continue;
                    }
                    let x = x + j;
                    let cc = match clip_mask {
                        Some(m) => clip_cov(Some(m), w, x, y0 + y),
                        None => 1.0,
                    };
                    if cc <= 0.0 {
                        continue;
                    }
                    dst[y * w + x] = src_over(dst[y * w + x], shadow_src(stored, cov, cc));
                }
            }
            x += 4;
        }
    }
    for (y, s) in run.rows() {
        for x in x_in_lo..x_in_hi {
            let cc = match clip_mask {
                Some(m) => clip_cov(Some(m), w, x, y0 + y),
                None => 1.0,
            };
            if cc <= 0.0 {
                continue;
            }
            dst[y * w + x] = src_over(dst[y * w + x], shadow_src(stored, s, cc));
        }
    }
}

/// Charge new clip storage to both query diagnostics and the public peak.
fn clip_alloc(bytes: u64, meter: Option<&FrameMeter>) {
    sel_alloc(bytes, meter);
    if let Some(meter) = meter {
        let live = meter.clip_live.fetch_add(bytes, Ordering::Relaxed) + bytes;
        meter.clip_peak.fetch_max(live, Ordering::Relaxed);
    }
}

/// Coverage borrowed from one raster window, never retained by the scene.
#[derive(Clone, Copy, Debug)]
pub enum ClipCoverage<'a> {
    /// Integer clip, without a raster allocation.
    Rect(IRect),
    /// Exact-area coverage of the current row window.
    Samples {
        /// Row-major coverage at the surface width.
        cells: &'a [f32],
        /// Device row represented by the first sample row.
        top: usize,
    },
}

/// Reused by a worker across items and bands; one slot per recursive window.
#[derive(Default)]
struct ClipScratch {
    cells: Vec<f32>,
    previous: Option<ClipRef>,
    rows: Range<usize>,
    width: usize,
}

impl ClipScratch {
    fn prepare<'a>(
        &'a mut self,
        clip: Option<&ClipRef>,
        rows: Range<usize>,
        width: usize,
        compiler: &mut Compiler,
        meter: Option<(&mut u64, &FrameMeter)>,
    ) -> Option<ClipCoverage<'a>> {
        let clip = clip?;
        match clip.as_ref() {
            ClipMask::Rect(rect) => Some(ClipCoverage::Rect(*rect)),
            ClipMask::Geometry { boundary, .. } => {
                if self.width != width
                    || self.rows != rows
                    || !self
                        .previous
                        .as_ref()
                        .is_some_and(|old| std::sync::Arc::ptr_eq(old, clip))
                {
                    let before =
                        compiler.heap_bytes() + (self.cells.capacity() * size_of::<f32>()) as u64;
                    self.cells.resize(width * rows.len(), 0.0);
                    self.cells.fill(0.0);
                    compiler.prepare(
                        boundary,
                        super::tiles::Fixed::default(),
                        rows.clone(),
                        width,
                    );
                    compiler.rows(rows.len(), |y, range, run| {
                        for (i, x) in range.enumerate() {
                            self.cells[y * width + x] = run.at(i);
                        }
                    });
                    if let Some((charged, meter)) = meter {
                        let bytes = compiler.heap_bytes()
                            + (self.cells.capacity() * size_of::<f32>()) as u64
                            - before;
                        clip_alloc(bytes, Some(meter));
                        *charged += bytes;
                    }
                    self.previous = Some(clip.clone());
                    self.rows = rows;
                    self.width = width;
                }
                Some(ClipCoverage::Samples {
                    cells: &self.cells,
                    top: self.rows.start,
                })
            }
        }
    }
}

/// The clip coverage of one device pixel in a prepared row window.
#[expect(
    clippy::cast_sign_loss,
    reason = "clip rect edges are clamped non-negative before indexing"
)]
fn clip_cov(clip: Option<ClipCoverage<'_>>, w: usize, px: usize, py: usize) -> f32 {
    match clip {
        None => 1.0,
        Some(ClipCoverage::Rect(r)) => f32::from(
            px >= r.x0.max(0) as usize
                && px < r.x1.max(0) as usize
                && py >= r.y0.max(0) as usize
                && py < r.y1.max(0) as usize,
        ),
        Some(ClipCoverage::Samples { cells, top }) => cells[(py - top) * w + px],
    }
}

/// The buffer at the top of the isolation stack — or the band's
/// framebuffer slice when the stack is empty — and the space its pixels
/// are stored in.
fn top<'a>(
    fb: &'a mut [[f32; 4]],
    fb_space: cherenkov::BlendSpace,
    stack: &'a mut [Plane],
) -> (&'a mut [[f32; 4]], cherenkov::BlendSpace) {
    stack.last_mut().map_or((fb, fb_space), |plane| {
        (plane.buf.as_mut_slice(), plane.space)
    })
}

/// `src_over` composite of premultiplied `src` onto `dst`.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn src_over(dst: [f32; 4], src: [f32; 4]) -> [f32; 4] {
    let inv = 1.0 - src[3];
    [
        src[0].mul_add(1.0, dst[0] * inv),
        src[1].mul_add(1.0, dst[1] * inv),
        src[2].mul_add(1.0, dst[2] * inv),
        src[3].mul_add(1.0, dst[3] * inv),
    ]
}

/// A 3×4 premultiplied colour matrix on `s` (filtrate `ColorMatrix`
/// layout): `dot(row, s)` per channel, alpha passes through.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn color_matrix(m: &[f32; 12], s: [f32; 4]) -> [f32; 4] {
    [
        s[0].mul_add(m[0], s[1].mul_add(m[1], s[2].mul_add(m[2], s[3] * m[3]))),
        s[0].mul_add(m[4], s[1].mul_add(m[5], s[2].mul_add(m[6], s[3] * m[7]))),
        s[0].mul_add(m[8], s[1].mul_add(m[9], s[2].mul_add(m[10], s[3] * m[11]))),
        s[3],
    ]
}

/// Evaluates an [`SdfEffect`] at pixel centre `(px + 0.5, py + 0.5)`
/// given the field's signed distance `d` and unit outward normal
/// `(nx, ny)` there: `Refraction` displaced-bilinear reads the capture,
/// `Rim` adds the highlight to the sampled colour (alpha unchanged —
/// gaining alpha would cancel under src-over).
#[expect(
    clippy::cast_precision_loss,
    reason = "pixels stay well below f32's integer bound"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn sdf_sample(
    capture: &Capture,
    sdf: &SdfEffect,
    (d, nx, ny): (f32, f32, f32),
    px: usize,
    py: usize,
    crow: usize,
) -> [f32; 4] {
    match sdf.kind {
        SdfKind::Refraction { depth, strength } => {
            let (qx, qy) = refracted((d, nx, ny), depth, strength, px, py);
            capture_sample(capture, qx, qy)
        }
        SdfKind::Rim { width, color, gain } => {
            let t = d.mul_add(width.recip(), 1.0).clamp(0.0, 1.0);
            let k = color[3] * gain * t * t;
            let s = pixel_sample(capture, px, py, crow);
            [
                color[0].mul_add(k, s[0]),
                color[1].mul_add(k, s[1]),
                color[2].mul_add(k, s[2]),
                s[3],
            ]
        }
        SdfKind::Level {
            depth,
            edge,
            interior,
        } => {
            let t = d.mul_add(depth.recip(), 1.0).clamp(0.0, 1.0);
            let level = (edge - interior).mul_add(t, interior);
            capture_sample_level(capture, px as f32 + 0.5, py as f32 + 0.5, level)
        }
    }
}

/// The refracted read point of pixel `(px, py)` given the field's
/// distance and unit normal there: `SdfKind::Refraction`'s displacement.
#[expect(clippy::cast_precision_loss, reason = "pixel coordinates are small")]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn refracted(
    (d, nx, ny): (f32, f32, f32),
    depth: f32,
    strength: f32,
    px: usize,
    py: usize,
) -> (f32, f32) {
    let t = d.mul_add(depth.recip(), 1.0).clamp(0.0, 1.0);
    (
        (nx * strength * t).mul_add(-t, px as f32 + 0.5),
        (ny * strength * t).mul_add(-t, py as f32 + 0.5),
    )
}

/// The distance, normal and union coverage of pixel `(px, py)`, shared
/// by capture planning and member compositing: the union field when the
/// member is in a union, the SDF effect's own clip distance otherwise.
#[expect(clippy::cast_precision_loss, reason = "pixel coordinates are small")]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn sample_field(
    effect: &SampleEffect,
    union: Option<&UnionSample>,
    px: usize,
    py: usize,
) -> ((f32, f32, f32), f32) {
    let (x, y) = (px as f32 + 0.5, py as f32 + 0.5);
    union.map_or_else(
        || match effect {
            SampleEffect::Sdf(sdf) => (box_sdf_at(&sdf.clip, x, y), 1.0),
            SampleEffect::None | SampleEffect::Color(_) => {
                unreachable!("only union members and SDF effects evaluate a field")
            }
        },
        |u| {
            let (d, nx, ny, own, width) = union_field_at(u, x, y);
            let coverage = own * (0.5 - (d - u.outer) / width).clamp(0.0, 1.0);
            ((d, nx, ny), coverage)
        },
    )
}

/// The union field at `(x, y)`: `(field, nx, ny, w_own, w)` — every
/// member's distance and gradient folded in ascending order with the
/// quadratic smin, the folded gradient's unit normal, this member's
/// ownership weight `a_ord / Σ_j a_j`, and the folded gradient's length
/// (the field's pixel width). `k == 0` (a lone `outer` member) folds to
/// the member's own field. `a_i = clamp(0.5 + f_i/|∇d₂ − ∇d_i|, 0, 1)`
/// with `f_i = d₂ − d_i` against the member's nearest competitor — a
/// hard step only where `|∇d₂ − ∇d_i| < 1e-6` (coincident shapes), an
/// exact `f_i == 0` tie going to the earliest member in paint order.
/// The same math as `union_field` in the GPU's `union.wgsl`, in f32.
// `const` items keep `clippy::inline_always` from firing here; the
// attribute is still required so the pixel loop compiles under the
// dispatched CPU level.
#[inline(always)]
fn union_field_at(
    union: &crate::render::lower::UnionSample,
    x: f32,
    y: f32,
) -> (f32, f32, f32, f32, f32) {
    const MAX: usize = cherenkov::BackdropUnion::MAX_MEMBERS as usize;
    // The planner enforces the cap. Initialize only the live members:
    // clearing three maximum-size arrays here otherwise costs a memory
    // primitive for every pixel, including footprint-planning samples.
    let mut fields = ArrayVec::<_, MAX>::new();
    for member in union.members.iter() {
        fields.push(box_sdf_at(member, x, y));
    }
    let count = fields.len();
    let ord = union.ord as usize;
    // Insertion sort of member indexes by distance in `total_cmp`
    // order — like the oracle's `sort_by` and the GPU fold: exact ties
    // keep paint order, and a −0.0/+0.0 pair resolves the same on every
    // engine.
    let mut order = ArrayVec::<usize, MAX>::new();
    for i in 0..count {
        order.push(i);
        let mut j = i;
        while j > 0 && fields[order[j - 1]].0.total_cmp(&fields[i].0) == std::cmp::Ordering::Greater
        {
            order[j] = order[j - 1];
            j -= 1;
        }
        order[j] = i;
    }
    let (mut field, gx, gy) = fields[order[0]];
    let mut grad = (gx, gy);
    for &j in order.iter().skip(1) {
        let (distance, gx, gy) = fields[j];
        let blend = (union.k - (distance - field)).max(0.0) / union.k;
        field = (blend * blend * union.k).mul_add(-0.25, field);
        let share = 0.5 * blend;
        grad.0 = share.mul_add(gx - grad.0, grad.0);
        grad.1 = share.mul_add(gy - grad.1, grad.1);
    }
    let width = grad.0.hypot(grad.1).max(1e-6);
    let (nx, ny) = (grad.0 / width, grad.1 / width);
    // `a_j` per member against its nearest competitor (`order[0]`, or
    // `order[1]` when `j` is the argmin itself).
    let w_own = if count == 1 {
        1.0
    } else {
        let mut sum = 0.0f32;
        let mut own = 0.0f32;
        for j in 0..count {
            let other = if order[0] == j { order[1] } else { order[0] };
            let f = fields[other].0 - fields[j].0;
            let slope = (fields[other].1 - fields[j].1).hypot(fields[other].2 - fields[j].2);
            let a = if slope < 1e-6 {
                if f > 0.0 || (f == 0.0 && j == order[0]) {
                    1.0
                } else {
                    0.0
                }
            } else {
                (0.5 + f / slope).clamp(0.0, 1.0)
            };
            sum += a;
            if j == ord {
                own = a;
            }
        }
        own / sum
    };
    (field, nx, ny, w_own, width)
}

/// The Euclidean length of `(x, y)` — WGSL's `length`.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn length(x: f32, y: f32) -> f32 {
    x.mul_add(x, y * y).sqrt()
}

/// The signed distance and unit outward normal of device point `(x, y)`
/// to `clip`'s boundary — the shader's `device_sdf` in f32: the point
/// mapped into box-local space by `inv`, the box's distance and outward
/// normal evaluated there, the normal mapped back by `inv`'s transpose
/// and normalised, and the distance divided by the same stretch.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn box_sdf_at(clip: &BoxClip, x: f32, y: f32) -> (f32, f32, f32) {
    let m = &clip.inv;
    let (px, py) = (
        m[0].mul_add(x, m[2].mul_add(y, m[4])),
        m[1].mul_add(x, m[3].mul_add(y, m[5])),
    );
    let (d, gx, gy) = clip.shape.sdf_sample(px, py);
    let (dx, dy) = (m[0].mul_add(gx, m[1] * gy), m[2].mul_add(gx, m[3] * gy));
    let len = length(dx, dy).max(1e-6);
    (d / len, dx / len, dy / len)
}

/// The plain sample of `capture` at device pixel `(px, py)`, whose kept
/// row starts at `crow`: the texel itself on a 1:1 capture, the bilinear
/// sample at the pixel centre on a reduced one.
#[expect(
    clippy::cast_precision_loss,
    reason = "pixels stay well below f32's integer bound"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn pixel_sample(capture: &Capture, px: usize, py: usize, crow: usize) -> [f32; 4] {
    if capture.scale.is_full() {
        capture.buf[crow + px - capture.x0]
    } else {
        capture_sample(capture, px as f32 + 0.5, py as f32 + 0.5)
    }
}

/// Bilinear sample of `capture` at device point `(x, y)`: `(x, y) · s` on
/// the capture grid, texel centres at integer + 0.5, clamped to the
/// capture region — the GPU's `backdrop_sample` convention.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "the coordinates are clamped into the capture first"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn capture_sample(capture: &Capture, x: f32, y: f32) -> [f32; 4] {
    let s = capture.scale.get();
    let fx = (x.mul_add(s, -(capture.x0 as f32)) - 0.5).clamp(0.0, capture.w as f32 - 1.0);
    let fy = (y.mul_add(s, -(capture.y0 as f32)) - 0.5).clamp(0.0, capture.rows as f32 - 1.0);
    let (x_lo, y_lo) = (fx.floor() as usize, fy.floor() as usize);
    let (x_hi, y_hi) = (
        (x_lo + 1).min(capture.w - 1),
        (y_lo + 1).min(capture.rows - 1),
    );
    let (tx, ty) = (fx - x_lo as f32, fy - y_lo as f32);
    let at = |x: usize, y: usize| capture.buf[y * capture.w + x];
    let (c00, c10, c01, c11) = (
        at(x_lo, y_lo),
        at(x_hi, y_lo),
        at(x_lo, y_hi),
        at(x_hi, y_hi),
    );
    let mix = |a: [f32; 4], b: [f32; 4], t: f32| {
        [
            (b[0] - a[0]).mul_add(t, a[0]),
            (b[1] - a[1]).mul_add(t, a[1]),
            (b[2] - a[2]).mul_add(t, a[2]),
            (b[3] - a[3]).mul_add(t, a[3]),
        ]
    };
    mix(mix(c00, c10, tx), mix(c01, c11, tx), ty)
}

/// Bilinear sample of `capture`'s level `k` at device point `(x, y)`:
/// `(x, y) · s / 2^k` on the level's grid, texel centres at integer +
/// 0.5, clamped to the kept rows — the GPU's `backdrop_sample_at`
/// convention.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "the coordinates are clamped into the capture first"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn capture_sample_at(capture: &Capture, x: f32, y: f32, k: u32) -> [f32; 4] {
    if k == 0 {
        return capture_sample(capture, x, y);
    }
    let level = &capture.levels[k as usize - 1];
    if level.rows == 0 {
        unreachable!(
            "capture_rows keeps every level's tap rows for each sampled device row, \
             so a sampled level holds at least one row"
        );
    }
    let div = (1u64 << k) as f32;
    let s = capture.scale.get();
    // Level-k texel coordinates relative to the region's origin; `y0`
    // shifts them to the kept rows' window.
    let fx = (x.mul_add(s, -(capture.x0 as f32)) / div - 0.5).clamp(0.0, level.w as f32 - 1.0);
    let fy = (y.mul_add(s, -(capture.ry0 as f32)) / div - 0.5 - level.y0 as f32)
        .clamp(0.0, level.rows as f32 - 1.0);
    let (x_lo, y_lo) = (fx.floor() as usize, fy.floor() as usize);
    let (x_hi, y_hi) = ((x_lo + 1).min(level.w - 1), (y_lo + 1).min(level.rows - 1));
    let (tx, ty) = (fx - x_lo as f32, fy - y_lo as f32);
    let at = |x: usize, y: usize| level.buf[y * level.w + x];
    let (c00, c10, c01, c11) = (
        at(x_lo, y_lo),
        at(x_hi, y_lo),
        at(x_lo, y_hi),
        at(x_hi, y_hi),
    );
    let mix = |a: [f32; 4], b: [f32; 4], t: f32| {
        [
            (b[0] - a[0]).mul_add(t, a[0]),
            (b[1] - a[1]).mul_add(t, a[1]),
            (b[2] - a[2]).mul_add(t, a[2]),
            (b[3] - a[3]).mul_add(t, a[3]),
        ]
    };
    mix(mix(c00, c10, tx), mix(c01, c11, tx), ty)
}

/// Trilinear sample of `capture`'s pyramid at device point `(x, y)`:
/// `level` clamped to `[0, n − 1]`, bilinear at `floor(level)` and
/// `ceil(level)` mixed by `fract(level)` — the GPU's
/// `backdrop_sample_level` convention.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "the level is clamped into the pyramid first"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn capture_sample_level(capture: &Capture, x: f32, y: f32, level: f32) -> [f32; 4] {
    let n = capture.levels.len() as u32 + 1;
    let lc = level.clamp(0.0, (n - 1) as f32);
    let k0 = lc.floor() as u32;
    let k1 = (k0 + 1).min(n - 1);
    let lo = capture_sample_at(capture, x, y, k0);
    let hi = capture_sample_at(capture, x, y, k1);
    let t = lc - k0 as f32;
    [
        (hi[0] - lo[0]).mul_add(t, lo[0]),
        (hi[1] - lo[1]).mul_add(t, lo[1]),
        (hi[2] - lo[2]).mul_add(t, lo[2]),
        (hi[3] - lo[3]).mul_add(t, lo[3]),
    ]
}

/// Convert a premultiplied pixel between linear and sRGB-encoded
/// storage.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn move_space<S: Simd>(
    simd: S,
    px: [f32; 4],
    from: cherenkov::BlendSpace,
    to: cherenkov::BlendSpace,
) -> [f32; 4] {
    if from == to {
        px
    } else if to == cherenkov::BlendSpace::SrgbEncoded {
        convert_packed::<S, true>(simd, px)
    } else {
        convert_packed::<S, false>(simd, px)
    }
}

/// A member draw's linear premultiplied source in the level's storage
/// space: members composite with each other in the level's space.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn member<S: Simd>(simd: S, px: [f32; 4], space: cherenkov::BlendSpace) -> [f32; 4] {
    move_space(simd, px, cherenkov::BlendSpace::Linear, space)
}

/// Composite `src` onto `dst` blending in `space`: each operand converts
/// from its storage space, the blend applies, and the result lands back
/// in the destination's storage space.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn composite<S: Simd>(
    simd: S,
    mode: cherenkov::BlendMode,
    space: cherenkov::BlendSpace,
    dst_space: cherenkov::BlendSpace,
    src_space: cherenkov::BlendSpace,
    dst: [f32; 4],
    src: [f32; 4],
) -> [f32; 4] {
    move_space(
        simd,
        super::blend::blend(
            mode,
            move_space(simd, dst, dst_space, space),
            move_space(simd, src, src_space, space),
        ),
        space,
        dst_space,
    )
}

/// Clip coverage already resolved to one row and run origin.
#[derive(Clone, Copy, Debug)]
pub struct ClipRun<'a> {
    span: ClipSpan<'a>,
}

#[derive(Clone, Copy, Debug)]
enum ClipSpan<'a> {
    Full,
    Rect { first: usize, last: usize },
    Samples(&'a [f32]),
}

impl<'a> ClipRun<'a> {
    /// Resolve the clip at `x0..` on device row `py` once per span.
    #[must_use]
    #[expect(
        clippy::cast_sign_loss,
        reason = "rectangle edges are clamped non-negative"
    )]
    pub fn new(clip: Option<&ClipCoverage<'a>>, w: usize, x0: usize, py: usize) -> Self {
        let span = match clip {
            None => ClipSpan::Full,
            Some(ClipCoverage::Rect(rect)) => {
                if py < rect.y0.max(0) as usize || py >= rect.y1.max(0) as usize {
                    ClipSpan::Rect { first: 0, last: 0 }
                } else {
                    ClipSpan::Rect {
                        first: (rect.x0.max(0) as usize).saturating_sub(x0),
                        last: (rect.x1.max(0) as usize).saturating_sub(x0),
                    }
                }
            }
            Some(ClipCoverage::Samples { cells, top }) => {
                let row = (py - top) * w;
                ClipSpan::Samples(&cells[row + x0..row + w])
            }
        };
        Self { span }
    }

    /// The clip coverage of the run's `i`th pixel.
    fn at(&self, i: usize) -> f32 {
        match self.span {
            ClipSpan::Full => 1.0,
            ClipSpan::Rect { first, last } => f32::from(i >= first && i < last),
            ClipSpan::Samples(cells) => cells[i],
        }
    }
}

/// Matrix columns operate on the packed pixel's RGB channels, preserving
/// `(m0 * r + m1 * g) + m2 * b` without fused operations.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn matrix_pixel<S: Simd>(simd: S, m: &[[f32; 3]; 3], rgb: [f32; 4]) -> f32x4<S> {
    let col0 = f32x4::load_array(simd, [m[0][0], m[1][0], m[2][0], 0.0]);
    let col1 = f32x4::load_array(simd, [m[0][1], m[1][1], m[2][1], 0.0]);
    let col2 = f32x4::load_array(simd, [m[0][2], m[1][2], m[2][2], 0.0]);
    simd.add_f32x4(
        simd.add_f32x4(
            simd.mul_f32x4(col0, simd.splat_f32x4(rgb[0])),
            simd.mul_f32x4(col1, simd.splat_f32x4(rgb[1])),
        ),
        simd.mul_f32x4(col2, simd.splat_f32x4(rgb[2])),
    )
}

/// filtrate-core's `srgb_encode` on four channels: the sRGB curve of each
/// linear magnitude, mirrored through zero, with `powf` evaluated as
/// `exp(γ⁻¹·ln x)` under the dispatched level.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn srgb_encode4<S: Simd>(simd: S, linear: f32x4<S>) -> f32x4<S> {
    let magnitude = simd.abs_f32x4(linear);
    let power = exp4(
        simd,
        simd.mul_f32x4(ln4(simd, magnitude), simd.splat_f32x4(SRGB_INVERSE_GAMMA)),
    );
    let curve = simd.mul_add_f32x4(
        power,
        simd.splat_f32x4(SRGB_SCALE),
        simd.splat_f32x4(-SRGB_OFFSET),
    );
    let line = simd.mul_f32x4(magnitude, simd.splat_f32x4(SRGB_SLOPE));
    simd.copysign_f32x4(
        simd.simd_gt_f32x4(magnitude, simd.splat_f32x4(SRGB_LINEAR_BREAK))
            .select(curve, line),
        linear,
    )
}

/// filtrate-core's `srgb_decode` on four channels, the inverse of
/// [`srgb_encode4`].
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn srgb_decode4<S: Simd>(simd: S, encoded: f32x4<S>) -> f32x4<S> {
    let magnitude = simd.abs_f32x4(encoded);
    let base = simd.div_f32x4(
        simd.add_f32x4(magnitude, simd.splat_f32x4(SRGB_OFFSET)),
        simd.splat_f32x4(SRGB_SCALE),
    );
    let curve = exp4(
        simd,
        simd.mul_f32x4(ln4(simd, base), simd.splat_f32x4(SRGB_GAMMA)),
    );
    let line = simd.div_f32x4(magnitude, simd.splat_f32x4(SRGB_SLOPE));
    simd.copysign_f32x4(
        simd.simd_gt_f32x4(magnitude, simd.splat_f32x4(SRGB_ENCODED_BREAK))
            .select(curve, line),
        encoded,
    )
}

/// Converts one packed pixel with the vector transfer functions. Zero
/// alpha returns before division or transfer evaluation; other alpha bits
/// are retained unchanged.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn convert_packed<S: Simd, const ENCODE: bool>(simd: S, pixel: [f32; 4]) -> [f32; 4] {
    let alpha = pixel[3];
    if alpha == 0.0 {
        return [0.0; 4];
    }
    let straight = simd
        .div_f32x4(
            f32x4::load_array(simd, [pixel[0], pixel[1], pixel[2], 0.0]),
            simd.splat_f32x4(alpha),
        )
        .to_array();
    let linear = if ENCODE {
        srgb_encode4(simd, matrix_pixel(simd, &P3_TO_SRGB, straight))
    } else {
        matrix_pixel(
            simd,
            &SRGB_TO_P3,
            srgb_decode4(simd, f32x4::load_array(simd, straight)).to_array(),
        )
    };
    let out = simd.mul_f32x4(linear, simd.splat_f32x4(alpha)).to_array();
    [out[0], out[1], out[2], alpha]
}

/// `matrix_pixel` on four pixels held one channel per vector, with the
/// same `(m0 * r + m1 * g) + m2 * b` order per output channel.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn matrix_quad<S: Simd>(simd: S, m: &[[f32; 3]; 3], rgb: [f32x4<S>; 3]) -> [f32x4<S>; 3] {
    [
        matrix_row(simd, &m[0], rgb),
        matrix_row(simd, &m[1], rgb),
        matrix_row(simd, &m[2], rgb),
    ]
}

/// One output channel of [`matrix_quad`].
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn matrix_row<S: Simd>(simd: S, row: &[f32; 3], rgb: [f32x4<S>; 3]) -> f32x4<S> {
    simd.add_f32x4(
        simd.add_f32x4(
            simd.mul_f32x4(simd.splat_f32x4(row[0]), rgb[0]),
            simd.mul_f32x4(simd.splat_f32x4(row[1]), rgb[1]),
        ),
        simd.mul_f32x4(simd.splat_f32x4(row[2]), rgb[2]),
    )
}

/// `convert_packed` on four pixels at once, one channel per vector, so
/// every lane of the transfer functions carries a pixel. Each pixel's
/// arithmetic is `convert_packed`'s. Returns the converted channel planes
/// for `f32x4::store_four_interleaved`.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn convert_quad<S: Simd, const ENCODE: bool>(simd: S, group: &[[f32; 4]; 4]) -> [f32x4<S>; 4] {
    let [r, g, b, alpha] = f32x4::load_four_interleaved(simd, group.as_flattened());
    let straight = [
        simd.div_f32x4(r, alpha),
        simd.div_f32x4(g, alpha),
        simd.div_f32x4(b, alpha),
    ];
    let rgb = if ENCODE {
        let [r, g, b] = matrix_quad(simd, &P3_TO_SRGB, straight);
        [
            srgb_encode4(simd, r),
            srgb_encode4(simd, g),
            srgb_encode4(simd, b),
        ]
    } else {
        let [r, g, b] = straight;
        matrix_quad(
            simd,
            &SRGB_TO_P3,
            [
                srgb_decode4(simd, r),
                srgb_decode4(simd, g),
                srgb_decode4(simd, b),
            ],
        )
    };
    let zero = simd.splat_f32x4(0.0);
    let transparent = simd.simd_eq_f32x4(alpha, zero);
    [
        transparent.select(zero, simd.mul_f32x4(rgb[0], alpha)),
        transparent.select(zero, simd.mul_f32x4(rgb[1], alpha)),
        transparent.select(zero, simd.mul_f32x4(rgb[2], alpha)),
        transparent.select(zero, alpha),
    ]
}

#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
#[expect(
    clippy::float_cmp,
    reason = "reuse a converted colour only when every source channel is equal"
)]
fn convert_span_in<S: Simd, const ENCODE: bool>(simd: S, pixels: &mut [[f32; 4]]) {
    let (chunks, tail) = pixels.as_chunks_mut::<4>();
    for group in chunks {
        // Solid fills and clear regions need one transfer evaluation, not
        // four identical evaluations. Varying pixels retain the wide path.
        if group[1] == group[0] && group[2] == group[0] && group[3] == group[0] {
            *group = [convert_packed::<S, ENCODE>(simd, group[0]); 4];
            continue;
        }
        let out = convert_quad::<S, ENCODE>(simd, group);
        f32x4::store_four_interleaved(out, group.as_flattened_mut());
    }
    for pixel in tail {
        *pixel = convert_packed::<S, ENCODE>(simd, *pixel);
    }
}

/// `convert_span_in` reading `src` and writing `dst`, so a caller that
/// must keep its source pixels converts without first copying them.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn convert_span_from<S: Simd, const ENCODE: bool>(simd: S, src: &[[f32; 4]], dst: &mut [[f32; 4]]) {
    assert_eq!(src.len(), dst.len(), "conversion run length mismatch");
    let (src_chunks, src_tail) = src.as_chunks::<4>();
    let (dst_chunks, dst_tail) = dst.as_chunks_mut::<4>();
    for (from, to) in src_chunks.iter().zip(dst_chunks) {
        let out = convert_quad::<S, ENCODE>(simd, from);
        f32x4::store_four_interleaved(out, to.as_flattened_mut());
    }
    for (from, to) in src_tail.iter().zip(dst_tail) {
        *to = convert_packed::<S, ENCODE>(simd, *from);
    }
}

/// `move_space` over packed pixels, with the same helper for full groups
/// and the tail. Equal spaces preserve every input bit.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
pub fn convert_span<S: Simd>(
    simd: S,
    pixels: &mut [[f32; 4]],
    from: cherenkov::BlendSpace,
    to: cherenkov::BlendSpace,
) {
    if from == to {
        return;
    }
    if to == cherenkov::BlendSpace::SrgbEncoded {
        convert_span_in::<S, true>(simd, pixels);
    } else {
        convert_span_in::<S, false>(simd, pixels);
    }
}

/// Converts `src`, stored in the other blend space, into `to` in `dst`:
/// `convert_span` out of place, with the same per-pixel arithmetic.
///
/// # Panics
/// When `src` and `dst` are not the same length.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn convert_span_into<S: Simd>(
    simd: S,
    src: &[[f32; 4]],
    dst: &mut [[f32; 4]],
    to: cherenkov::BlendSpace,
) {
    if to == cherenkov::BlendSpace::SrgbEncoded {
        convert_span_from::<S, true>(simd, src, dst);
    } else {
        convert_span_from::<S, false>(simd, src, dst);
    }
}

/// Packed source-over, preserving the coverage/clip multiply order and
/// separate destination multiply and add. A blocked clip does not touch
/// destination bits; NaN clip coverage follows the computed path.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn blend_packed<S: Simd, const SRGB: bool>(
    simd: S,
    dst: &mut [f32; 4],
    src: [f32; 4],
    coverage: f32,
    clip: f32,
) {
    if clip <= 0.0 {
        return;
    }
    let scaled = simd.mul_f32x4(
        simd.mul_f32x4(f32x4::load_array(simd, src), simd.splat_f32x4(coverage)),
        simd.splat_f32x4(clip),
    );
    let scaled = if SRGB {
        f32x4::load_array(simd, convert_packed::<S, true>(simd, scaled.to_array()))
    } else {
        scaled
    };
    let inverse = simd.splat_f32x4(1.0 - scaled.to_array()[3]);
    *dst = simd
        .add_f32x4(
            scaled,
            simd.mul_f32x4(f32x4::load_array(simd, *dst), inverse),
        )
        .to_array();
}

/// The clip's presence is selected once per span; every pixel remains
/// packed throughout the arithmetic.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn composite_span_in<S: Simd, C: Source, const SRGB: bool, const CLIPPED: bool>(
    simd: S,
    dst: &mut [[f32; 4]],
    src: &[[f32; 4]],
    cov: C,
    clip: ClipRun<'_>,
) {
    let mut pos = 0;
    while pos + 4 <= dst.len() {
        for lane in 0..4 {
            let i = pos + lane;
            let cc = if CLIPPED { clip.at(i) } else { 1.0 };
            blend_packed::<S, SRGB>(simd, &mut dst[i], src[i], cov.at(i), cc);
        }
        pos += 4;
    }
    for i in pos..dst.len() {
        let cc = if CLIPPED { clip.at(i) } else { 1.0 };
        blend_packed::<S, SRGB>(simd, &mut dst[i], src[i], cov.at(i), cc);
    }
}

/// Composites a run of evaluated `src` pixels over `dst`.
///
/// One pixel is `src_over(dst, member(src * cov * cc, space))`, `cc` the
/// run's clip coverage, pixels with `cc <= 0.0` untouched: a draw's
/// per-pixel expression, unchanged. `dst`, `src` and `cov` are the same
/// length.
///
/// # Panics
/// When `dst`, `src` and `cov` are not the same length.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
pub fn composite_span<S: Simd, C: Source>(
    simd: S,
    dst: &mut [[f32; 4]],
    src: &[[f32; 4]],
    cov: C,
    clip: ClipRun<'_>,
    space: cherenkov::BlendSpace,
) {
    assert_eq!(dst.len(), src.len(), "composite run length mismatch");
    cov.check_len(dst.len());
    match (
        space == cherenkov::BlendSpace::SrgbEncoded,
        !matches!(clip.span, ClipSpan::Full),
    ) {
        (true, true) => composite_span_in::<S, C, true, true>(simd, dst, src, cov, clip),
        (true, false) => composite_span_in::<S, C, true, false>(simd, dst, src, cov, clip),
        (false, true) => composite_span_in::<S, C, false, true>(simd, dst, src, cov, clip),
        (false, false) => composite_span_in::<S, C, false, false>(simd, dst, src, cov, clip),
    }
}

/// `composite_span` specialized to a solid paint `color` (premultiplied
/// linear): the packed source is constant, so the span never reads
/// evaluated pixels. An opaque `color` (`a == 1`) additionally
/// splits each maximal `cov == 1 && cc == 1` run and stores
/// `member(color)` — converted once — directly, without reading `dst`;
/// partial boundaries run the shared packed kernel, which produces the
/// same `member(color)` value at full coverage anyway.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
pub fn composite_solid<S: Simd, C: Source>(
    simd: S,
    dst: &mut [[f32; 4]],
    cov: C,
    clip: ClipRun<'_>,
    color: [f32; 4],
    space: cherenkov::BlendSpace,
) {
    cov.check_len(dst.len());
    // The space conversion divides out alpha, so the converted colour
    // scaled by coverage is the converted coverage-scaled colour: one
    // conversion serves every pixel of the span, and the loop itself is
    // the same in either space.
    let stored = if space == cherenkov::BlendSpace::SrgbEncoded {
        convert_packed::<S, true>(simd, color)
    } else {
        color
    };
    if matches!(clip.span, ClipSpan::Full) {
        composite_solid_in::<S, C, false>(simd, dst, cov, clip, stored);
    } else {
        composite_solid_in::<S, C, true>(simd, dst, cov, clip, stored);
    }
}

/// Stores four identical solid pixels from precomputed channel planes.
/// This opaque-only store keeps the interleaved SIMD instruction without
/// transposing the ordinary composition or conversion paths.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn store_solid_block<S: Simd>(planes: [f32x4<S>; 4], pixels: &mut [[f32; 4]; 4]) {
    // SAFETY: `[[f32; 4]; 4]` is layout-identical to `[f32; 16]`.
    f32x4::store_four_interleaved(planes, unsafe {
        &mut *pixels.as_mut_ptr().cast::<[f32; 16]>()
    });
}

/// `composite_solid`'s per-space loop; see it for the contract.
#[expect(clippy::float_cmp, reason = "full coverage is the exact value 1.0")]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level"
)]
#[inline(always)]
fn composite_solid_in<S: Simd, C: Source, const CLIPPED: bool>(
    simd: S,
    dst: &mut [[f32; 4]],
    cov: C,
    clip: ClipRun<'_>,
    stored: [f32; 4],
) {
    // The conversion keeps alpha, so `stored` is opaque exactly when the
    // paint colour is.
    let opaque = stored[3] == 1.0;
    let stored_planes = [
        simd.splat_f32x4(stored[0]),
        simd.splat_f32x4(stored[1]),
        simd.splat_f32x4(stored[2]),
        simd.splat_f32x4(stored[3]),
    ];
    if opaque && cov.full() && !CLIPPED {
        let (groups, tail) = dst.as_chunks_mut::<4>();
        for group in groups {
            store_solid_block(stored_planes, group);
        }
        for pixel in tail {
            *pixel = stored;
        }
        return;
    }
    let count = dst.len();
    let mut pos = 0;
    while pos < count {
        if opaque && cov.at(pos) == 1.0 && (!CLIPPED || clip.at(pos) == 1.0) {
            let start = pos;
            while pos < count && cov.at(pos) == 1.0 && (!CLIPPED || clip.at(pos) == 1.0) {
                pos += 1;
            }
            let (groups, tail) = dst[start..pos].as_chunks_mut::<4>();
            for group in groups {
                store_solid_block(stored_planes, group);
            }
            for pixel in tail {
                *pixel = stored;
            }
        } else if pos + 4 <= count {
            for lane in 0..4 {
                let i = pos + lane;
                let cc = if CLIPPED { clip.at(i) } else { 1.0 };
                blend_packed::<S, false>(simd, &mut dst[i], stored, cov.at(i), cc);
            }
            pos += 4;
        } else {
            let cc = if CLIPPED { clip.at(pos) } else { 1.0 };
            blend_packed::<S, false>(simd, &mut dst[pos], stored, cov.at(pos), cc);
            pos += 1;
        }
    }
}

/// Shades one positive compiler run under the dispatched CPU level.
#[expect(
    clippy::too_many_arguments,
    clippy::inline_always,
    clippy::cast_sign_loss,
    reason = "integer crop inlines into the caller; each pixel kernel has its own SIMD boundary"
)]
#[inline(always)]
fn shade_run<S: Simd>(
    simd: S,
    dst: &mut [[f32; 4]],
    cov: Run<'_>,
    range: Range<usize>,
    py: usize,
    w: usize,
    paint: &PaintData,
    clip: Option<&ClipCoverage<'_>>,
    space: cherenkov::BlendSpace,
    src: &mut [MaybeUninit<[f32; 4]>],
) {
    let (range, cov, clip) = if let Some(ClipCoverage::Rect(rect)) = clip {
        if py < rect.y0.max(0) as usize || py >= rect.y1.max(0) as usize {
            return;
        }
        let start = range.start.max(rect.x0.max(0) as usize);
        let end = range.end.min(rect.x1.max(0) as usize);
        if start >= end {
            return;
        }
        let cov = cov.slice(start - range.start..end - range.start);
        (start..end, cov, None)
    } else {
        (range, cov, clip)
    };
    if let PaintData::Solid(color) = paint {
        let clip = ClipRun::new(clip, w, range.start, py);
        let dst = &mut dst[range];
        let color = *color;
        // Isolate the solid kernel from the paint evaluators' register pressure.
        // Capture the span context by value across the SIMD boundary.
        simd.vectorize(
            #[inline(always)]
            move || match cov {
                Run::Uniform(c) => composite_solid(simd, dst, c, clip, color, space),
                Run::Samples(c) => composite_solid(simd, dst, c, clip, color, space),
                Run::Slice(c) => composite_solid(simd, dst, c, clip, color, space),
            },
        );
        return;
    }
    simd.vectorize(
        #[inline(always)]
        move || {
            let mut x = range.start;
            while x < range.end {
                let n = (range.end - x).min(SPAN_CHUNK);
                let src = paint.eval_span(simd, x, py, &mut src[..n]);
                let clip = ClipRun::new(clip, w, x, py);
                let dst = &mut dst[x..x + n];
                match cov.slice(x - range.start..x - range.start + n) {
                    Run::Uniform(c) => composite_span(simd, dst, src, c, clip, space),
                    Run::Samples(c) => composite_span(simd, dst, src, c, clip, space),
                    Run::Slice(c) => composite_span(simd, dst, src, c, clip, space),
                }
                x += n;
            }
        },
    );
}

/// `(draws, edges)` stats for an item list.
fn stats(items: &[Item]) -> (u32, u32) {
    let (mut draws, mut edges) = (0_u32, 0_u32);

    for item in items {
        if matches!(item, Item::Silhouette { .. } | Item::Project(_)) {
            draws += 1;
        }
        if let Item::Draw { edges: e, .. } = item {
            draws += 1;
            edges += u32::try_from(e.len()).unwrap_or(u32::MAX);
        }
    }
    (draws, edges)
}

/// The rows `union`'s capture covers for the surface band
/// `[y0, y1)`: the union's rows within `reach` of the band.
fn kept_rows(union: &IRect, reach: usize, band: (usize, usize), h: usize) -> (usize, usize) {
    let kept0 = usize::try_from(union.y0)
        .unwrap_or(0)
        .max(band.0.saturating_sub(reach));
    let kept1 = usize::try_from(union.y1)
        .unwrap_or(0)
        .min(band.1.saturating_add(reach).min(h));
    (kept0, kept1)
}

/// A band's working background; the public output remains linear P3.
/// When every root composite is normal sRGB, keep that working space
/// through the band instead of converting the destination at every group.
/// Backdrop captures and filter windows retain their linear root canvas.
#[derive(Clone, Copy)]
struct Background {
    color: [f32; 4],
    space: cherenkov::BlendSpace,
}

impl Background {
    fn new(
        level: fearless_simd::Level,
        items: &[Item],
        clear: [f32; 4],
        has_backdrop: bool,
    ) -> Self {
        use cherenkov::{BlendMode, BlendSpace};
        let linear = Self {
            color: clear,
            space: BlendSpace::Linear,
        };
        if has_backdrop || items.is_empty() {
            return linear;
        }
        let mut depth = 0;
        for item in items {
            match item {
                Item::PushIsolate { space } => {
                    if depth == 0 && *space != BlendSpace::SrgbEncoded {
                        return linear;
                    }
                    depth += 1;
                }
                Item::PopIsolate { blend, .. } => {
                    depth -= 1;
                    if depth == 0 && *blend != (BlendMode::Normal, BlendSpace::SrgbEncoded) {
                        return linear;
                    }
                }
                Item::PushFilter { .. } => return linear,
                _ if depth == 0 => return linear,
                _ => {}
            }
        }
        Self {
            color: cherenkov_simd::dispatch!(level, simd =>
                move_space(simd, clear, BlendSpace::Linear, BlendSpace::SrgbEncoded)),
            space: BlendSpace::SrgbEncoded,
        }
    }
}

/// Shades one band of `slice` rows (`w * bh` pixels, cleared first),
/// starting at device row `y0`.
///
/// With no `Capture` item the band renders `w * bh` pixels as before.
/// When a capture exists, the rows its window needs extend past the
/// band, so the item list runs over an expanded window up to the last
/// top-level capture, then collapses to the band for the rest.
#[expect(
    clippy::too_many_arguments,
    reason = "the band context travels together"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn shade(
    level: fearless_simd::Level,
    items: &[Item],
    index: &FrameIndex,
    background: Background,
    slice: &mut [[f32; 4]],
    w: usize,
    y0: usize,
    h: usize,
    scratch: &mut Scratch<'_>,
    peak: Option<&std::sync::atomic::AtomicU64>,
    has_backdrop: bool,
    meter: Option<&FrameMeter>,
) -> Result<(), cherenkov::RenderError> {
    let bh = slice.len() / w;
    for (_, capture) in scratch.captures.drain() {
        scratch.buffers.give_color(capture.buf);
    }
    scratch.buffers.meter.reset();
    slice.fill(background.color);
    let surface = (y0, y0 + bh);
    // A surface that used no backdrop group cannot contain a `Capture`,
    // so the scan is skipped outright.
    let top = if has_backdrop {
        top_captures(level, items, surface, (w, h), meter)
    } else {
        TopCaptures::new()
    };
    let window = top
        .iter()
        .filter_map(|(_, rows)| rows.as_ref())
        .fold(surface, |window, rows| {
            (window.0.min(rows.window.0), window.1.max(rows.window.1))
        });
    let last_capture = top
        .iter()
        .rev()
        .find_map(|(i, rows)| rows.as_ref().map(|_| *i));
    let coverage = &mut scratch.coverage;
    let mut pass = BandScratch {
        stack: &mut scratch.stack,
        buffers: &mut scratch.buffers,
        captures: &mut scratch.captures,
    };
    let result = if let Some(last) = last_capture {
        shade_windowed(
            level,
            items,
            index,
            background.color,
            slice,
            w,
            surface,
            h,
            (window, last),
            coverage,
            (&mut pass, &top),
            meter,
        )
    } else {
        shade_plain(
            level,
            items,
            index,
            slice,
            background.space,
            w,
            surface,
            h,
            coverage,
            (&mut pass, &top),
            meter,
        )
    };
    for plane in scratch.stack.drain(..) {
        scratch.buffers.give_color(plane.buf);
    }
    if scratch.buffers.meter.captured
        && let Some(peak) = peak
    {
        peak.fetch_max(
            (scratch.buffers.meter.delta_peak() * size_of::<[f32; 4]>()) as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
    }
    if result.is_ok() && background.space != cherenkov::BlendSpace::Linear {
        cherenkov_simd::dispatch!(level, simd => convert_span(
            simd, slice, background.space, cherenkov::BlendSpace::Linear));
    }
    result
}

/// Plans the top-level captures (captures inside filter scopes are
/// planned by the scope's own windowed run) for the surface band
/// `surface`, last to first. A later capture reads the rows its window
/// covers, including what this group's members drew there when one of
/// them samples before that capture's item; such a capture's window
/// widens the rows this capture covers past the band.
fn top_captures(
    level: fearless_simd::Level,
    items: &[Item],
    surface: (usize, usize),
    size: (usize, usize),
    meter: Option<&FrameMeter>,
) -> TopCaptures {
    // (capture item, its group, the first item sampling the group)
    let mut plan: Vec<(usize, u64, usize)> = Vec::new();
    let mut scope_end = 0;
    for (i, item) in items.iter().enumerate() {
        match item {
            Item::PushFilter { end, .. } if i >= scope_end => {
                scope_end = usize::try_from(*end).unwrap_or(items.len());
            }
            Item::Capture(capture) if i >= scope_end => {
                plan.push((i, capture.group, usize::MAX));
            }
            Item::Sample { group, .. } => {
                if let Some((_, _, first)) = plan
                    .iter_mut()
                    .find(|(_, g, first)| g == group && *first == usize::MAX)
                {
                    *first = i;
                }
            }
            _ => {}
        }
    }
    let mut top: TopCaptures = plan.iter().map(|&(i, _, _)| (i, None)).collect();
    for k in (0..plan.len()).rev() {
        let (i, _, first) = plan[k];
        let need = top[k + 1..]
            .iter()
            .filter(|(j, _)| first < *j)
            .filter_map(|(_, rows)| rows.as_ref())
            .fold(surface, |need, rows| {
                (need.0.min(rows.window.0), need.1.max(rows.window.1))
            });
        let Item::Capture(capture) = &items[i] else {
            unreachable!("only capture items are planned");
        };
        top[k].1 = capture_rows(level, capture, items, need, size, meter);
    }
    top
}

/// The band's pass with no capture: `run` over the caller's slice.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
#[expect(
    clippy::too_many_arguments,
    reason = "the band context travels together"
)]
fn shade_plain(
    level: fearless_simd::Level,
    items: &[Item],
    index: &FrameIndex,
    slice: &mut [[f32; 4]],
    space: cherenkov::BlendSpace,
    w: usize,
    surface: (usize, usize),
    h: usize,
    acc: &mut Compiler,
    (scratch, top): (&mut BandScratch<'_>, &[(usize, Option<CaptureRows>)]),
    meter: Option<&FrameMeter>,
) -> Result<(), cherenkov::RenderError> {
    let (y0, _) = surface;
    let b = y0 / BAND_H;
    let mut band = Band {
        fb: slice,
        w,
        y0,
        space,
    };
    run_at(
        level,
        items,
        index,
        index.items(b, items),
        items.len(),
        &mut band,
        acc,
        scratch.stack,
        scratch.buffers,
        &mut FrameCtx {
            level,
            surface,
            h,
            captures: scratch.captures,
            top,
            meter,
        },
        0,
    )
}

/// The band's pass with top-level captures: `run` over the expanded
/// `window` rows up to the `last` capture item, then over the band for
/// the rest.
///
/// The window covers the band plus every capture's window rows
/// ([`CaptureRows::window`]); the capture canvas clamps to its region
/// inside it. On success the central rows copy back into `slice` and
/// each live isolation buffer crops to band size.
#[expect(
    clippy::too_many_arguments,
    reason = "the band's captured state travels together"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn shade_windowed(
    level: fearless_simd::Level,
    items: &[Item],
    index: &FrameIndex,
    clear: [f32; 4],
    slice: &mut [[f32; 4]],
    w: usize,
    surface: (usize, usize),
    h: usize,
    ((win0, win1), last): ((usize, usize), usize),
    acc: &mut Compiler,
    (scratch, top): (&mut BandScratch<'_>, &[(usize, Option<CaptureRows>)]),
    meter: Option<&FrameMeter>,
) -> Result<(), cherenkov::RenderError> {
    let (y0, y1) = surface;
    let bh = y1 - y0;
    let win1 = win1.min(h);
    let rows = win1 - win0;
    let mut window = scratch.buffers.take_color(w * rows);
    window.fill(clear);
    let mut band = Band {
        fb: &mut window,
        w,
        y0: win0,
        space: cherenkov::BlendSpace::Linear,
    };
    // The window's item selection lives in the depth-0 slot; the run
    // and its nested scopes take slots 1 and deeper.
    let mut sel = scratch.buffers.take_sel_items(0, meter);
    index.query_items(0..last + 1, (win0, win1), &mut sel);
    scratch.buffers.charge_sel_items(0, &sel, meter);
    let first_pass = run_at(
        level,
        items,
        index,
        index.selected(&sel, items),
        last + 1,
        &mut band,
        acc,
        scratch.stack,
        scratch.buffers,
        &mut FrameCtx {
            level,
            surface,
            h,
            captures: scratch.captures,
            top,
            meter,
        },
        1,
    );
    scratch.buffers.give_sel_items(0, sel);
    if first_pass.is_ok() {
        // Collapse to the band: the central rows go to the output slice
        // and each live isolation buffer shrinks to band size.
        let first = (y0 - win0) * w;
        slice.copy_from_slice(&window[first..first + w * bh]);
        for plane in scratch.stack.iter_mut() {
            let mut cropped = scratch.buffers.take_color(w * bh);
            cropped.copy_from_slice(&plane.buf[first..first + w * bh]);
            scratch
                .buffers
                .give_color(std::mem::replace(&mut plane.buf, cropped));
        }
    }
    scratch.buffers.give_color(window);
    first_pass?;
    let mut band = Band {
        fb: slice,
        w,
        y0,
        space: cherenkov::BlendSpace::Linear,
    };
    // The aligned band's bins, restricted to the items after the last
    // top-level capture.
    run_at(
        level,
        items,
        index,
        index.items_after(y0 / BAND_H, items, last + 1),
        items.len(),
        &mut band,
        acc,
        scratch.stack,
        scratch.buffers,
        &mut FrameCtx {
            level,
            surface,
            h,
            captures: scratch.captures,
            top,
            meter,
        },
        0,
    )
}

/// A pass's exclusive output, split only by unique atomic band claims.
///
/// The input borrow lasts through the joined raster tasks. Claims never
/// create a shared reference to pixel contents or reuse a claimed band.
struct BandQueue<'a, T> {
    selected: Option<&'a cherenkov::damage::Region>,
    count: usize,
    output: *mut T,
    len: usize,
    chunk_len: NonZeroUsize,
    next: AtomicUsize,
    borrow: PhantomData<&'a mut [T]>,
}

// SAFETY: the queue owns the exclusive input borrow. Its only pixel access
// hands each uniquely claimed, disjoint band to one task; sharing the queue
// never shares pixel references. Moving those elements between tasks needs
// T: Send, not T: Sync. The borrow outlives all claims and joined tasks.
unsafe impl<T: Send> Sync for BandQueue<'_, T> {}

impl<'a, T> BandQueue<'a, T> {
    fn new(
        output: &'a mut [T],
        chunk_len: NonZeroUsize,
        selected: Option<&'a cherenkov::damage::Region>,
    ) -> Self {
        let count = selected.map_or_else(
            || output.len().div_ceil(chunk_len.get()),
            |selected| usize::try_from(selected.pixel_area()).expect("band count fits usize"),
        );
        Self {
            selected,
            count,
            output: output.as_mut_ptr(),
            len: output.len(),
            chunk_len,
            next: AtomicUsize::new(0),
            borrow: PhantomData,
        }
    }

    #[expect(
        clippy::mut_from_ref,
        reason = "unique atomic claims partition the exclusively borrowed output"
    )]
    fn claim(&self) -> Option<(usize, &mut [T])> {
        let ordinal = self
            .next
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                (next < self.count).then(|| next + 1)
            })
            .ok()?;
        let index = self.selected.map_or(ordinal, |selected| {
            let mut remaining = ordinal;
            for rect in selected.rectangles() {
                let length = rect.height() as usize;
                if remaining < length {
                    return rect.y0() as usize + remaining;
                }
                remaining -= length;
            }
            unreachable!("claimed band lies inside selected coverage")
        });
        let start = index * self.chunk_len.get();
        let len = self.chunk_len.get().min(self.len - start);
        // SAFETY: try_update claims each ordinal exactly once. The selected
        // region contains disjoint bands within this output, so mapping an
        // ordinal to its band preserves uniqueness and keeps start < len.
        // The exclusive input borrow and this queue borrow keep the
        // allocation alive for the result.
        let band = unsafe { std::slice::from_raw_parts_mut(self.output.add(start), len) };
        Some((index, band))
    }
}

/// Runs at most min(bands, workers) tasks, each reusing one stack-owned
/// Scratch. A claim transfers one band without copying output or allocating.
/// The transient pixel bound is workers × (band + Σ filter windows at the
/// deepest nesting), with each window sized w × (bh + 2·apron).
fn for_each_band<T: Send>(
    output: &mut [T],
    w: usize,
    h: usize,
    edges: Option<usize>,
    meter: &FrameMeter,
    selected: Option<&cherenkov::damage::Region>,
    shade_band: impl Fn(usize, &mut [T], &mut Scratch<'_>) -> Result<(), cherenkov::RenderError> + Sync,
) -> Result<(), cherenkov::RenderError> {
    if output.is_empty() || h == 0 {
        return Ok(());
    }
    let chunk_len =
        NonZeroUsize::new(w * h.min(BAND_H)).expect("non-empty raster output has non-zero width");
    let queue = BandQueue::new(output, chunk_len, selected);
    let tasks = queue.count.min(rayon::current_num_threads());
    (0..tasks).into_par_iter().try_for_each(|_| {
        let Some((mut index, mut band)) = queue.claim() else {
            return Ok(());
        };
        let mut scratch = Scratch::new(Some(meter), w, edges);
        loop {
            shade_band(index * BAND_H, band, &mut scratch)?;
            match queue.claim() {
                Some(next) => (index, band) = next,
                None => return Ok(()),
            }
        }
    })
}

/// Rasterizes the whole surface's items into `fb`, dynamically claiming
/// whole bands. Each task reuses its coverage, isolation and filter-window
/// buffers until no bands remain. All tasks join before the pass returns.
#[expect(
    clippy::too_many_arguments,
    reason = "the band context travels together"
)]
pub fn render_bands(
    level: fearless_simd::Level,
    items: &[Item],
    index: &mut FrameIndex,
    clear: [f32; 4],
    fb: &mut [[f32; 4]],
    w: usize,
    h: usize,
    peak: Option<&std::sync::atomic::AtomicU64>,
    has_backdrop: bool,
    meter: &mut FrameMeter,
    selected: Option<&cherenkov::damage::Region>,
) -> Result<(u32, u32), cherenkov::RenderError> {
    index.rebuild(items, h);
    meter.record_index(index.heap_bytes());
    let meter = &*meter;
    let index = &*index;
    let stats = stats(items);
    let background = Background::new(level, items, clear, has_backdrop);
    for_each_band(
        fb,
        w,
        h,
        index.coverage_capacity(has_backdrop),
        meter,
        selected,
        |y0, band, scratch| {
            shade(
                level,
                items,
                index,
                background,
                band,
                w,
                y0,
                h,
                scratch,
                peak,
                has_backdrop,
                Some(meter),
            )
        },
    )?;
    Ok(stats)
}

/// Rasterizes into `out`, converting each finished band to `f16`. Shading
/// runs at full `f32`; the store rounds once, at the output boundary.
#[expect(
    clippy::too_many_arguments,
    reason = "the band context travels together"
)]
pub fn render_bands_f16(
    level: fearless_simd::Level,
    items: &[Item],
    index: &mut FrameIndex,
    clear: [f32; 4],
    out: &mut [[half::f16; 4]],
    w: usize,
    h: usize,
    peak: Option<&std::sync::atomic::AtomicU64>,
    has_backdrop: bool,
    meter: &mut FrameMeter,
    selected: Option<&cherenkov::damage::Region>,
) -> Result<(u32, u32), cherenkov::RenderError> {
    index.rebuild(items, h);
    meter.record_index(index.heap_bytes());
    let meter = &*meter;
    let index = &*index;
    let stats = stats(items);
    let background = Background::new(level, items, clear, has_backdrop);
    for_each_band(
        out,
        w,
        h,
        index.coverage_capacity(has_backdrop),
        meter,
        selected,
        |y0, out_slice, scratch| {
            let mut band_px = scratch.buffers.take_color(out_slice.len());
            let result = shade(
                level,
                items,
                index,
                background,
                &mut band_px,
                w,
                y0,
                h,
                scratch,
                peak,
                has_backdrop,
                Some(meter),
            );
            if result.is_ok() {
                store_f16(out_slice, &band_px);
            }
            scratch.buffers.give_color(band_px);
            result
        },
    )?;
    Ok(stats)
}

/// Rasterizes band by band in row order, delivering each finished band to
/// `sink` in the target's format. No full-frame buffer exists.
///
/// `emit` is the surface's conversion buffer for `LinearF16` output.
#[expect(
    clippy::too_many_arguments,
    reason = "the band context travels together"
)]
pub fn render_bands_stream(
    level: fearless_simd::Level,
    items: &[Item],
    clear: [f32; 4],
    size: (usize, usize),
    emit: &mut Vec<[half::f16; 4]>,
    format: OffscreenFormat,
    sink: &mut dyn FnMut(BandOut<'_>),
    peak: Option<&std::sync::atomic::AtomicU64>,
    has_backdrop: bool,
    index: &FrameIndex,
    meter: &mut FrameMeter,
) -> Result<(u32, u32), cherenkov::RenderError> {
    let stats = stats(items);
    let background = Background::new(level, items, clear, has_backdrop);
    let (w, h) = size;
    meter.record_index(index.heap_bytes());
    let meter = &*meter;
    let mut scratch = Scratch::new(Some(meter), w, index.coverage_capacity(has_backdrop));
    let mut band_px = scratch.buffers.take_color(w * h.min(BAND_H));
    let mut y0 = 0;
    while y0 < h {
        let bh = (h - y0).min(BAND_H);
        if let Err(error) = shade(
            level,
            items,
            index,
            background,
            &mut band_px[..w * bh],
            w,
            y0,
            h,
            &mut scratch,
            peak,
            has_backdrop,
            Some(meter),
        ) {
            scratch.buffers.give_color(band_px);
            return Err(error);
        }
        let y = u32::try_from(y0).unwrap_or(u32::MAX);
        match format {
            OffscreenFormat::LinearF32 => sink(BandOut {
                y,
                pixels: BandPixels::F32(&band_px[..w * bh]),
            }),
            OffscreenFormat::LinearF16 => {
                emit.clear();
                emit.resize(w * bh, [half::f16::ZERO; 4]);
                store_f16(emit, &band_px[..w * bh]);
                sink(BandOut {
                    y,
                    pixels: BandPixels::F16(emit.as_slice()),
                });
            }
        }
        y0 += bh;
    }
    scratch.buffers.give_color(band_px);
    // `scratch`'s Drop releases the charged query capacity.
    drop(scratch);
    debug_assert_eq!(
        meter.query_live.load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    Ok(stats)
}

/// Converts `src` to `f16` over `dst` with one slice conversion: `half`'s
/// arch dispatch vectorizes the store where the target has f16 conversion
/// hardware, and each element rounds exactly as `f16::from_f32` does.
fn store_f16(dst: &mut [[half::f16; 4]], src: &[[f32; 4]]) {
    dst.as_flattened_mut()
        .convert_from_f32_slice(src.as_flattened());
}

const fn slice_len(w: usize, bh: usize) -> usize {
    w * bh
}

/// `run` exactly once per dispatched CPU level: the band passes and
/// `run`'s own recursion enter through this boundary, so `run`'s
/// `#[inline(always)]` body exists once per level rather than once per
/// call site.
#[expect(
    clippy::too_many_arguments,
    reason = "the band context travels together"
)]
#[inline(never)]
fn run_at(
    level: fearless_simd::Level,
    items: &[Item],
    index: &FrameIndex,
    sel: Selection<'_>,
    limit: usize,
    band: &mut Band<'_>,
    acc: &mut Compiler,
    stack: &mut Vec<Plane>,
    buffers: &mut Buffers,
    ctx: &mut FrameCtx<'_>,
    depth: usize,
) -> Result<(), cherenkov::RenderError> {
    cherenkov_simd::dispatch!(level, simd => run(simd, items, index, sel, limit, band, acc, stack, buffers, ctx, depth))
}

#[expect(
    clippy::too_many_lines,
    reason = "keeps ordered item processing and recursive filter scopes together"
)]
#[expect(
    clippy::too_many_arguments,
    reason = "the band context travels together"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn run<S: Simd>(
    simd: S,
    items: &[Item],
    index: &FrameIndex,
    sel: Selection<'_>,
    limit: usize,
    band: &mut Band<'_>,
    acc: &mut Compiler,
    stack: &mut Vec<Plane>,
    buffers: &mut Buffers,
    ctx: &mut FrameCtx<'_>,
    depth: usize,
) -> Result<(), cherenkov::RenderError> {
    let bh = band.fb.len() / band.w;
    let mut clips = buffers.clips.pop().unwrap_or_default();
    let h = ctx.h;
    // Item indexes below `skip` were consumed by a filter scope's
    // nested run.
    let mut skip = 0usize;
    for i in sel {
        if i < skip {
            continue;
        }
        match &items[i] {
            Item::Draw {
                edges,
                offset,
                bbox,
                paint,
                clip,
            } => {
                let clip = clips.prepare(
                    clip.as_ref(),
                    band.y0..band.y0 + bh,
                    band.w,
                    acc,
                    ctx.meter.map(|m| (&mut buffers.clip_charge, m)),
                );
                band.draw(
                    simd,
                    acc,
                    stack,
                    (edges, *offset),
                    *bbox,
                    paint,
                    clip.as_ref(),
                );
            }
            Item::PushIsolate { space } => stack.push(Plane {
                buf: buffers.take_color(slice_len(band.w, bh)),
                space: *space,
            }),
            Item::PopIsolate {
                opacity,
                blend,
                clip,
            } => {
                let Some(plane) = stack.pop() else {
                    continue;
                };
                band.composite_isolate(
                    simd,
                    (&plane.buf, plane.space),
                    *opacity,
                    *blend,
                    clips.prepare(
                        clip.as_ref(),
                        band.y0..band.y0 + bh,
                        band.w,
                        acc,
                        ctx.meter.map(|m| (&mut buffers.clip_charge, m)),
                    ),
                    stack,
                );
                buffers.give_color(plane.buf);
            }
            Item::PushFilter { end, apron, space } => {
                let scope_end = usize::try_from(*end)
                    .map_err(|_| cherenkov::RenderError::Render("invalid filter scope".into()))?;
                if scope_end <= i || scope_end >= limit {
                    return Err(cherenkov::RenderError::Render(
                        "invalid filter scope".into(),
                    ));
                }
                let Some(Item::PopFilter {
                    filter,
                    opacity,
                    blend,
                    clip,
                }) = items.get(scope_end)
                else {
                    return Err(cherenkov::RenderError::Render(
                        "invalid filter scope".into(),
                    ));
                };
                let y1 = band.y0 + bh;
                let top = band.y0.saturating_sub(*apron);
                let bottom = y1.saturating_add(*apron).min(h);
                let mut window = buffers.take_color(band.w * (bottom - top));
                let coverage = buffers.take_coverage(
                    band.w,
                    index.coverage_capacity(true).unwrap_or(0),
                    bottom - top,
                );
                let mut nested = buffers.take_sel_items(depth, ctx.meter);
                index.query_items(i + 1..scope_end, (top, bottom), &mut nested);
                buffers.charge_sel_items(depth, &nested, ctx.meter);
                let nested_result = {
                    let mut filter_band = Band {
                        fb: &mut window,
                        w: band.w,
                        y0: top,
                        space: *space,
                    };
                    let mut filter_acc = coverage;
                    let mut filter_stack = Vec::new();
                    let result = run_at(
                        ctx.level,
                        items,
                        index,
                        index.selected(&nested, items),
                        scope_end,
                        &mut filter_band,
                        &mut filter_acc,
                        &mut filter_stack,
                        buffers,
                        ctx,
                        depth + 1,
                    );
                    for plane in filter_stack {
                        buffers.give_color(plane.buf);
                    }
                    buffers.give_coverage(filter_acc);
                    result
                };
                buffers.give_sel_items(depth, nested);
                if let Err(error) = nested_result {
                    buffers.give_color(window);
                    return Err(error);
                }
                if let Err(error) =
                    apply_filter(simd, filter, &mut window, *space, top, (band.w, h))
                {
                    buffers.give_color(window);
                    return Err(error);
                }
                let first = (band.y0 - top) * band.w;
                let central = &window[first..first + band.fb.len()];
                band.composite_isolate(
                    simd,
                    (central, *space),
                    *opacity,
                    *blend,
                    clips.prepare(
                        clip.as_ref(),
                        band.y0..band.y0 + bh,
                        band.w,
                        acc,
                        ctx.meter.map(|m| (&mut buffers.clip_charge, m)),
                    ),
                    stack,
                );
                buffers.give_color(window);
                skip = scope_end + 1;
            }
            Item::PopFilter { .. } => {
                return Err(cherenkov::RenderError::Render(
                    "unpaired filter scope".into(),
                ));
            }
            Item::Capture(capture) => {
                let rows = match ctx.top.iter().find(|(j, _)| *j == i) {
                    Some((_, rows)) => *rows,
                    None => capture_rows(
                        ctx.level,
                        capture,
                        items,
                        ctx.surface,
                        (band.w, h),
                        ctx.meter,
                    ),
                };
                if let Some(rows) = rows {
                    capture_band(simd, band, stack.as_slice(), buffers, ctx, capture, rows)?;
                }
            }
            Item::Sample {
                group,
                bounds,
                clip,
                effect,
                union,
                space,
            } => {
                if let Some(capture) = ctx.captures.get(group) {
                    band.sample(
                        simd,
                        capture,
                        *bounds,
                        clips.prepare(
                            clip.as_ref(),
                            band.y0..band.y0 + bh,
                            band.w,
                            acc,
                            ctx.meter.map(|m| (&mut buffers.clip_charge, m)),
                        ),
                        effect,
                        union.as_ref(),
                        *space,
                        stack.as_mut_slice(),
                    );
                }
            }
            Item::Shadow {
                rbox,
                radii,
                sigma_eff,
                color,
                bbox,
                clip,
            } => band.shadow(
                simd,
                stack,
                rbox,
                radii,
                *sigma_eff,
                color,
                *bbox,
                clips.prepare(
                    clip.as_ref(),
                    band.y0..band.y0 + bh,
                    band.w,
                    acc,
                    ctx.meter.map(|m| (&mut buffers.clip_charge, m)),
                ),
            ),
            Item::Project(item) => {
                let clip = clips.prepare(
                    item.clip.as_ref(),
                    band.y0..band.y0 + bh,
                    band.w,
                    acc,
                    ctx.meter.map(|m| (&mut buffers.clip_charge, m)),
                );
                band.project(simd, stack, item, clip);
            }
            Item::Silhouette { slot, paint, clip } => {
                band.glyph(
                    simd,
                    stack,
                    slot,
                    0,
                    0,
                    paint,
                    clips
                        .prepare(
                            clip.as_ref(),
                            band.y0..band.y0 + bh,
                            band.w,
                            acc,
                            ctx.meter.map(|m| (&mut buffers.clip_charge, m)),
                        )
                        .as_ref(),
                );
            }
            Item::Glyph {
                slot,
                x,
                y,
                paint,
                clip,
            } => band.glyph(
                simd,
                stack,
                slot,
                *x,
                *y,
                paint,
                clips
                    .prepare(
                        clip.as_ref(),
                        band.y0..band.y0 + bh,
                        band.w,
                        acc,
                        ctx.meter.map(|m| (&mut buffers.clip_charge, m)),
                    )
                    .as_ref(),
            ),
        }
    }
    if !clips.cells.is_empty() {
        let before = buffers.clips.capacity();
        buffers.clips.push(clips);
        let bytes = ((buffers.clips.capacity() - before) * size_of::<ClipScratch>()) as u64;
        clip_alloc(bytes, ctx.meter);
        buffers.clip_charge += bytes;
    }
    Ok(())
}

/// Runs `filter` over `pixels` stored in `space`. A filter works in the
/// linear working space whatever space its scope stores: sRGB-encoded
/// pixels are decoded before it and encoded again after it.
fn apply_filter<S: Simd>(
    simd: S,
    (filter, params): &FrameFilter,
    pixels: &mut [[f32; 4]],
    space: cherenkov::BlendSpace,
    top: usize,
    size: (usize, usize),
) -> Result<(), cherenkov::RenderError> {
    let encoded = space == cherenkov::BlendSpace::SrgbEncoded;
    if encoded {
        convert_span(
            simd,
            pixels,
            cherenkov::BlendSpace::SrgbEncoded,
            cherenkov::BlendSpace::Linear,
        );
    }
    filter
        .apply(
            params,
            &WorkingSpace::LINEAR_DISPLAY_P3,
            &mut CpuImage {
                pixels: &mut *pixels,
                top,
                size,
            },
        )
        .map_err(|error| match error {
            CpuFilterError::GpuImage { .. } => {
                cherenkov::RenderError::Unsupported(crate::names::FILTER_GPU_IMAGE)
            }
        })?;
    if encoded {
        convert_span(
            simd,
            pixels,
            cherenkov::BlendSpace::Linear,
            cherenkov::BlendSpace::SrgbEncoded,
        );
    }
    Ok(())
}

/// Runs one backdrop group's capture for this band: copies the device
/// rows under the chain's window of the nearest semantic level with the
/// trailing `flatten` looked-through levels composited raw over it,
/// resolves them onto the capture grid when the capture is reduced,
/// applies the group's chain, and keeps the `rows.kept` texel rows for
/// this band's samples.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "device spans of texels inside the surface are small and non-negative"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn capture_band<S: Simd>(
    simd: S,
    band: &Band<'_>,
    stack: &[Plane],
    buffers: &mut Buffers,
    ctx: &mut FrameCtx<'_>,
    item: &CaptureItem,
    rows: CaptureRows,
) -> Result<(), cherenkov::RenderError> {
    let (kept0, kept1) = rows.kept;
    let (w, h) = (band.w, ctx.h);
    let region = item.region;
    let rx0 = usize::try_from(region.x0).unwrap_or(0);
    let rx1 = usize::try_from(region.x1).unwrap_or(0).min(w);
    let ry0 = usize::try_from(region.y0).unwrap_or(0);
    let ry1 = usize::try_from(region.y1).unwrap_or(0);
    let rw = rx1.saturating_sub(rx0);
    // The texel window the chain reads: `kept` plus `apron` rows, clamped
    // to the region.
    let win0 = kept0.saturating_sub(item.apron).max(ry0);
    let win1 = kept1.saturating_add(item.apron).min(ry1);
    let texel_rows = win1.saturating_sub(win0);
    if rw == 0 || texel_rows == 0 {
        return Ok(());
    }
    // The device rows and columns under the window's texels.
    let reduced = !item.scale.is_full();
    let s = f64::from(item.scale.get());
    let ((d0, d1), (c0, c1)) = if reduced {
        (
            (
                span(win0, s, h).0.floor() as usize,
                span(win1 - 1, s, h).1.ceil() as usize,
            ),
            (
                span(rx0, s, w).0.floor() as usize,
                span(rx1 - 1, s, w).1.ceil() as usize,
            ),
        )
    } else {
        ((win0, win1), (rx0, rx1))
    };
    let bh = band.fb.len() / w;
    if d0 < band.y0 || d1 > band.y0 + bh {
        return Err(cherenkov::RenderError::Render(
            "backdrop capture reads outside the run window".into(),
        ));
    }
    let cw = c1 - c0;
    let mut flat = buffers.take_color(cw * (d1 - d0));
    let space = match flatten(band, stack, item.flatten, (d0, d1), (c0, c1), &mut flat) {
        Ok(space) => space,
        Err(error) => {
            buffers.give_color(flat);
            return Err(error);
        }
    };
    let mut canvas = if reduced {
        let mut canvas = buffers.take_color(rw * texel_rows);
        let mut across = buffers.take_color(rw * (d1 - d0));
        resolve(
            Resolved {
                src: &flat,
                origin: (c0, d0),
                width: cw,
            },
            (&mut across, &mut canvas),
            (rx0, win0, rw),
            (w, h),
            s,
        );
        buffers.give_color(across);
        buffers.give_color(flat);
        canvas
    } else {
        flat
    };
    if let Some(filter) = &item.filter {
        apply_filter(
            simd,
            filter,
            &mut canvas,
            space,
            win0 - ry0,
            (rw, ry1 - ry0),
        )?;
    }
    let kept_rows = kept1 - kept0;
    let mut buf = buffers.take_color(rw * kept_rows);
    buf.copy_from_slice(&canvas[(kept0 - win0) * rw..(kept0 - win0) * rw + rw * kept_rows]);
    // A levelled group's pyramid over the chain's output: `canvas` is
    // level 0, holding the `win0..win1` window.
    let levels = capture_pyramid(&canvas, item, &rows, (rw, ry0, ry1, win0), buffers);
    buffers.give_color(canvas);
    buffers.meter.captured = true;
    ctx.captures.insert(
        item.group,
        Capture {
            buf,
            scale: item.scale,
            y0: kept0,
            rows: kept_rows,
            x0: rx0,
            w: rw,
            ry0,
            levels,
            device_rows: rows.device,
            device_cols: (c0, c1),
            space,
        },
    );
    Ok(())
}

/// Builds levels `1..item.levels` of a levelled capture: level `k` texel
/// `(x, y)` is the mean of level `k − 1` texels `(2x..=2x+1, 2y..=2y+1)`,
/// a partial box at the spec edge averaging the texels present — the
/// GPU reduce's exact 2×2 box. Each level's source is the previous
/// level's kept rows (`canvas` is level 0's, holding the `win0..win1`
/// texel window).
fn capture_pyramid(
    canvas: &[[f32; 4]],
    item: &CaptureItem,
    rows: &CaptureRows,
    (rw, ry0, ry1, win0): (usize, usize, usize, usize),
    buffers: &mut Buffers,
) -> Vec<CaptureLevel> {
    let mut levels: Vec<CaptureLevel> = Vec::new();
    if item.levels <= 1 {
        return levels;
    }
    // `sh`/`sw` are the source level's spec extent; `gy0` its grid
    // origin in absolute rows (`ry0 / 2^{k−1}`, exact since the region
    // is `2^{n−1}`-aligned); `src_y0` the first row its buf actually
    // holds.
    let (mut sw, mut sh, mut gy0, mut src_y0) = (rw, ry1 - ry0, ry0, win0);
    for (k, &range) in (1..item.levels as usize).zip(rows.levels.iter()) {
        let dw = sw.div_ceil(2);
        let (r0, r1) = range;
        let mut dst = buffers.take_color(r1.saturating_sub(r0) * dw);
        if r0 < r1 {
            let src: &[[f32; 4]] = levels
                .last()
                .map_or_else(|| canvas, |prev| prev.buf.as_slice());
            reduce_rows(src, sw, (src_y0, gy0 + sh), (r0, r1), &mut dst);
        }
        src_y0 = r0;
        gy0 >>= 1;
        sh = sh.div_ceil(2);
        sw = dw;
        levels.push(CaptureLevel {
            buf: dst,
            // The kept window relative to the level's grid origin
            // (`ry0 / 2^k`), which `capture_sample_at` subtracts.
            y0: r0 - (ry0 >> k),
            rows: r1.saturating_sub(r0),
            w: dw,
        });
    }
    levels
}

/// Rows `r0..r1` of the next pyramid level into `dst`, reduced from
/// `src`: the source level's rows from `src_y0` on, `sw` texels wide,
/// on a grid whose rows end at `src_end`. Texel `(x, r)` is the mean of
/// the source texels `(2x..=2x+1, 2r..=2r+1)` inside the grid — a
/// partial box at the right edge, the bottom edge or the corner averages
/// the texels present.
#[expect(
    clippy::cast_precision_loss,
    reason = "a partial 2×2 box holds at most 4 texels"
)]
fn reduce_rows(
    src: &[[f32; 4]],
    sw: usize,
    (src_y0, src_end): (usize, usize),
    (r0, r1): (usize, usize),
    dst: &mut [[f32; 4]],
) {
    let dw = sw.div_ceil(2);
    for r in r0..r1 {
        let (y0, y1) = (2 * r, (2 * r + 1).min(src_end - 1));
        for x in 0..dw {
            let (x0, x1) = (2 * x, (2 * x + 1).min(sw - 1));
            let mut acc = [0f32; 4];
            for row in y0..=y1 {
                for xx in x0..=x1 {
                    let c = src[(row - src_y0) * sw + xx];
                    acc[0] += c[0];
                    acc[1] += c[1];
                    acc[2] += c[2];
                    acc[3] += c[3];
                }
            }
            let n = ((y1 - y0 + 1) * (x1 - x0 + 1)) as f32;
            dst[(r - r0) * dw + x] = [acc[0] / n, acc[1] / n, acc[2] / n, acc[3] / n];
        }
    }
}

/// Copies the device `rows × cols` of the nearest semantic level below
/// the trailing `count` looked-through levels into `out` and composites
/// those levels over it as each would pop at full opacity — the oracle's
/// `flattened` — and returns the semantic level's space. Every
/// looked-through level is `Normal`-blended and stored in the root's
/// linear space (layers never sit inside an encoded group scope), so
/// each composites source-over.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn flatten(
    band: &Band<'_>,
    stack: &[Plane],
    count: usize,
    (d0, d1): (usize, usize),
    (c0, c1): (usize, usize),
    out: &mut [[f32; 4]],
) -> Result<cherenkov::BlendSpace, cherenkov::RenderError> {
    if count > stack.len() {
        return Err(cherenkov::RenderError::Render(
            "backdrop capture flatten underflow".into(),
        ));
    }
    let (w, cw) = (band.w, c1 - c0);
    let base = stack.len() - count;
    // The nearest semantic level: the framebuffer when every live level
    // is looked through, else the level below the flattened ones.
    let (src, space): (&[[f32; 4]], _) = if base == 0 {
        (band.fb, band.space)
    } else {
        (&stack[base - 1].buf, stack[base - 1].space)
    };
    for row in d0..d1 {
        let dst = &mut out[(row - d0) * cw..(row - d0) * cw + cw];
        dst.copy_from_slice(&src[(row - band.y0) * w + c0..(row - band.y0) * w + c0 + cw]);
    }
    for level in &stack[base..] {
        for row in d0..d1 {
            for px in 0..cw {
                let dst = &mut out[(row - d0) * cw + px];
                *dst = src_over(*dst, level.buf[(row - band.y0) * w + c0 + px]);
            }
        }
    }
    Ok(space)
}

/// Capture texel `i`'s device span `[i/s, (i+1)/s)` at scale `s`, clipped
/// to `[0, end)`.
#[expect(
    clippy::cast_precision_loss,
    reason = "texel indices and surface sizes are far below 2^53"
)]
fn span(i: usize, s: f64, end: usize) -> (f64, f64) {
    (i as f64 / s, ((i + 1) as f64 / s).min(end as f64))
}

/// Device pixels a reduced capture resolves from.
#[derive(Clone, Copy)]
struct Resolved<'a> {
    /// The pixels, `width` per row.
    src: &'a [[f32; 4]],
    /// The device position of `src[0]`.
    origin: (usize, usize),
    /// Row stride.
    width: usize,
}

/// One axis of the resolve's box: each device pixel under texel `i`'s
/// span at scale `s`, clipped to `[0, end)`, with its overlap weight.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "spans lie inside the surface; weights are in [0, 1]"
)]
fn taps(i: usize, s: f64, end: usize) -> impl Iterator<Item = (usize, f32)> {
    let (lo, hi) = span(i, s, end);
    let len = hi - lo;
    (lo.floor() as usize..hi.ceil() as usize).map(move |k| {
        let k0 = k as f64;
        (k, (((k0 + 1.0).min(hi) - k0.max(lo)) / len) as f32)
    })
}

/// Resolves `from` onto the capture grid at scale `s`: `out` receives
/// `width` texels per row starting at texel `(tx, ty)`, each the
/// area-weighted mean of the device pixels under its span clipped to
/// `extent` — rows across first into `across` (`from`'s rows × `width`),
/// then down.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn resolve(
    from: Resolved<'_>,
    (across, out): (&mut [[f32; 4]], &mut [[f32; 4]]),
    (tx, ty, width): (usize, usize, usize),
    extent: (usize, usize),
    s: f64,
) {
    let (ox, oy) = from.origin;
    for (src, dst) in from
        .src
        .chunks_exact(from.width)
        .zip(across.chunks_exact_mut(width))
    {
        for (i, texel) in dst.iter_mut().enumerate() {
            let mut acc = [0.0f32; 4];
            for (k, weight) in taps(tx + i, s, extent.0) {
                for (a, c) in acc.iter_mut().zip(src[k - ox]) {
                    *a = weight.mul_add(c, *a);
                }
            }
            *texel = acc;
        }
    }
    for (j, dst) in out.chunks_exact_mut(width).enumerate() {
        for (i, texel) in dst.iter_mut().enumerate() {
            let mut acc = [0.0f32; 4];
            for (k, weight) in taps(ty + j, s, extent.1) {
                for (a, c) in acc.iter_mut().zip(across[(k - oy) * width + i]) {
                    *a = weight.mul_add(c, *a);
                }
            }
            *texel = acc;
        }
    }
}

/// One band's rasterization state.
struct Band<'a> {
    /// The band's framebuffer rows.
    fb: &'a mut [[f32; 4]],
    /// Surface width.
    w: usize,
    /// Device y of the band's first row.
    y0: usize,
    /// The space the framebuffer stores (`Linear` for the surface, the
    /// scope's declared space inside a filtered scope).
    space: cherenkov::BlendSpace,
}

impl Band<'_> {
    /// Queries and rasterizes one draw's visible boundaries, placed at
    /// `offset`, into the top isolation buffer.
    #[expect(
        clippy::too_many_arguments,
        reason = "the draw context travels together"
    )]
    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level so mul_add is one instruction"
    )]
    #[inline(always)]
    fn draw<S: Simd>(
        &mut self,
        simd: S,
        acc: &mut Compiler,
        stack: &mut Vec<Plane>,
        (edges, offset): (&EdgeTiles, super::tiles::Fixed),
        bbox: crate::render::lower::DrawBox,
        paint: &PaintData,
        clip: Option<&ClipCoverage<'_>>,
    ) {
        let bh = self.fb.len() / self.w;
        let first = usize::from(bbox.y0).max(self.y0);
        let last = usize::from(bbox.y1).min(self.y0 + bh);
        if first >= last {
            return;
        }
        acc.prepare(edges, offset, first..last, self.w);
        let (dst, space) = top(&mut *self.fb, self.space, stack.as_mut_slice());
        let mut src = [const { MaybeUninit::<[f32; 4]>::uninit() }; SPAN_CHUNK];
        acc.rows(last - first, |y, range, run| {
            let py = first + y;
            let row = (py - self.y0) * self.w;
            shade_run(
                simd,
                &mut dst[row..row + self.w],
                run,
                range,
                py,
                self.w,
                paint,
                clip,
                space,
                &mut src,
            );
        });
    }

    /// Rasterizes a blurred rounded box: analytic coverage per pixel in
    /// `bbox` ∩ band.
    #[expect(
        clippy::cast_precision_loss,
        clippy::suboptimal_flops,
        clippy::too_many_arguments,
        clippy::too_many_lines,
        reason = "pixel indices and band offsets are far below 2^24 and \
                  the quadrature weights mirror the reference formula"
    )]
    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level so mul_add is one instruction"
    )]
    #[inline(always)]
    fn shadow<S: Simd>(
        &mut self,
        simd: S,
        stack: &mut Vec<Plane>,
        rbox: &[f32; 4],
        radii: &[f32; 4],
        sigma_eff: f32,
        color: &[f32; 4],
        bbox: IRect,
        clip: Option<ClipCoverage<'_>>,
    ) {
        let bh = self.fb.len() / self.w;
        let (y_lo, y_hi) = (
            usize::try_from(bbox.y0)
                .unwrap_or(0)
                .saturating_sub(self.y0)
                .min(bh),
            usize::try_from(bbox.y1)
                .unwrap_or(0)
                .saturating_sub(self.y0)
                .min(bh),
        );
        let (x_lo, x_hi) = (
            usize::try_from(bbox.x0).unwrap_or(0).min(self.w),
            usize::try_from(bbox.x1).unwrap_or(0).min(self.w),
        );
        if y_lo >= y_hi || x_lo >= x_hi {
            return;
        }
        let (cx, cy) = (rbox[0], rbox[1]);
        let half = [rbox[2], rbox[3]];
        let sigma = sigma_eff;
        let inv_sqrt2_sigma = 1.0 / (sigma * std::f32::consts::SQRT_2);
        // Rect-clip rows touch only `cx_lo..cx_hi`; a coverage clip stays
        // a per-pixel sample.
        let (cx_lo, cx_hi, clip_mask) = match clip {
            Some(ClipCoverage::Rect(r)) => (
                usize::try_from(r.x0).unwrap_or(0).clamp(x_lo, x_hi),
                usize::try_from(r.x1).unwrap_or(0).clamp(x_lo, x_hi),
                None,
            ),
            Some(mask @ ClipCoverage::Samples { .. }) => (x_lo, x_hi, Some(mask)),
            None => (x_lo, x_hi, None),
        };
        if cx_lo >= cx_hi && clip_mask.is_none() {
            return;
        }
        // The y quadrature the GPU shader uses (`shadow` in
        // gpu/src/render/shader.wgsl): per row, the ±3σ window clipped to
        // the box's ±half[1], one analytic `erf` sample for the straight
        // band |y| <= hy - rmax inside the window, and an 8-node
        // Gauss–Legendre rule (`shadow::GL8`) over each corner band the
        // window reaches.
        let rmax = radii.iter().copied().fold(0.0_f32, f32::max);
        let band = (half[1] - rmax).max(0.0);
        // Per row the quadrature's x-integral saturates except near the
        // left/right edges: `erf` reaches ±1 within `margin` of an edge,
        // so interior columns all evaluate to the same `s` (the sum of
        // in-box sample weights) and only the two edge bands pay the
        // per-sample `erf` sum. For `sigma`-wide margins the tail error
        // is `erf(5)-1 < 1e-11`.
        let margin = 5.0 * sigma * std::f32::consts::SQRT_2;
        // Consecutive rows with the same x intervals — every row whose
        // window misses the corner bands, and every row whose window
        // covers a whole corner band — form a run that shares the
        // intervals' x integrals; only the weights differ per row.
        let mut samples = [shadow::Sample::default(); shadow::MAX_SAMPLES];
        let mut run = shadow::Run::new();
        let mut integrals = shadow::Integrals::new();
        // The per-pixel quadrature runs inside `vectorize` so it compiles
        // under the dispatched level's target features — a second closure
        // on this path (a `map_or` or `get_or_insert_with` body) would be
        // compiled without them and call the FMA intrinsic out of line.
        simd.vectorize(
            #[inline(always)]
            || {
                for y in y_lo..y_hi {
                    let py = (self.y0 + y) as f32 + 0.5 - cy;
                    // The row's y window: `py ± 3σ` clipped to the box.
                    let lo = (py - 3.0 * sigma).max(-half[1]);
                    let hi = (py + 3.0 * sigma).min(half[1]);
                    if hi <= lo {
                        continue;
                    }
                    // The row's sample table: left/right x-edges per in-box
                    // sample, their weights, the interior-coverage sum `s`, and
                    // the most-inset edges bounding the interior zone.
                    let mut n = 0_usize;
                    let mut s = 0.0_f32;
                    let (mut xlm, mut xrm) = (f32::MIN, f32::MAX);
                    // The window's straight-band part is one sample whose
                    // weight is the Gaussian's exact integral over it.
                    let ya = lo.max(-band);
                    let yb = hi.min(band);
                    if yb > ya {
                        let ends = erf4(
                            simd,
                            f32x4::load_array(
                                simd,
                                [
                                    (yb - py) * inv_sqrt2_sigma,
                                    (ya - py) * inv_sqrt2_sigma,
                                    0.0,
                                    0.0,
                                ],
                            ),
                        )
                        .to_array();
                        samples[n] = shadow::Sample {
                            left: -half[0],
                            right: half[0],
                            weight: 0.5 * (ends[0] - ends[1]),
                        };
                        (xlm, xrm) = (-half[0], half[0]);
                        s += samples[n].weight;
                        n += 1;
                    }
                    // Each corner band the window reaches gets the
                    // shader's 8-node Gauss–Legendre rule; a node's x
                    // edges come from the corner inset at its row.
                    for (a, b, rl, rr) in [
                        (lo, hi.min(-band), radii[0], radii[1]),
                        (lo.max(band), hi, radii[3], radii[2]),
                    ] {
                        if b <= a {
                            continue;
                        }
                        let mid = a.midpoint(b);
                        let hw = 0.5 * (b - a);
                        for (node, w) in shadow::GL8 {
                            for sign in [-1.0_f32, 1.0] {
                                let yi = mid + sign * hw * node;
                                let ay = yi.abs();
                                samples[n] = shadow::Sample {
                                    left: -half[0] + corner_inset(rl, ay - (half[1] - rl)),
                                    right: half[0] - corner_inset(rr, ay - (half[1] - rr)),
                                    weight: w * hw * gaussian(yi - py, sigma),
                                };
                                xlm = xlm.max(samples[n].left);
                                xrm = xrm.min(samples[n].right);
                                s += samples[n].weight;
                                n += 1;
                            }
                        }
                    }
                    if n == 0 || s <= 0.0 {
                        continue;
                    }
                    if !run.continues(y, &samples[..n]) {
                        if !run.is_empty() {
                            let (dst, space) = top(&mut *self.fb, self.space, stack.as_mut_slice());
                            shadow_run(
                                simd,
                                dst,
                                self.w,
                                self.y0,
                                cx,
                                inv_sqrt2_sigma,
                                margin,
                                clip_mask,
                                &member(simd, *color, space),
                                (cx_lo, cx_hi),
                                &run,
                                &mut integrals,
                            );
                        }
                        run.start(y, &samples[..n], xlm, xrm);
                    }
                    run.push(s, &samples[..n]);
                }
                if !run.is_empty() {
                    let (dst, space) = top(&mut *self.fb, self.space, stack.as_mut_slice());
                    shadow_run(
                        simd,
                        dst,
                        self.w,
                        self.y0,
                        cx,
                        inv_sqrt2_sigma,
                        margin,
                        clip_mask,
                        &member(simd, *color, space),
                        (cx_lo, cx_hi),
                        &run,
                        &mut integrals,
                    );
                }
            },
        );
    }

    /// Composites `capture`'s rows over the band's top inside `bounds`,
    /// under `clip` — the member's backdrop sample — blending in
    /// `member_space`.
    /// Samples `capture` under `clip` with the member's `effect`
    /// (`SampleEffect::None` reads the capture unchanged) at full
    /// strength — the member's own scope attenuates it at composite.
    /// Writes are `clip`-coverage-gated: nothing lands outside the
    /// member clip.
    #[expect(
        clippy::too_many_arguments,
        reason = "one sample item's fields plus the band's plane stack"
    )]
    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level so mul_add is one instruction"
    )]
    #[inline(always)]
    fn sample<S: Simd>(
        &mut self,
        simd: S,
        capture: &Capture,
        bounds: IRect,
        clip: Option<ClipCoverage<'_>>,
        effect: &SampleEffect,
        union: Option<&UnionSample>,
        member_space: cherenkov::BlendSpace,
        stack: &mut [Plane],
    ) {
        let bh = self.fb.len() / self.w;
        let y_lo = usize::try_from(bounds.y0)
            .unwrap_or(0)
            .max(self.y0)
            .max(capture.device_rows.0);
        let y_hi = usize::try_from(bounds.y1)
            .unwrap_or(0)
            .min(self.y0 + bh)
            .min(capture.device_rows.1);
        let x_lo = usize::try_from(bounds.x0)
            .unwrap_or(0)
            .max(capture.device_cols.0)
            .min(self.w);
        let x_hi = usize::try_from(bounds.x1)
            .unwrap_or(0)
            .min(capture.device_cols.1)
            .min(self.w);
        if y_lo >= y_hi || x_lo >= x_hi {
            return;
        }
        let needs_field = union.is_some() || matches!(effect, SampleEffect::Sdf(_));
        let (dst, space) = top(&mut *self.fb, self.space, stack);
        for py in y_lo..y_hi {
            let row = (py - self.y0) * self.w;
            // The kept row's start, read only by a 1:1 capture's texel
            // reads, whose device rows are its texel rows.
            let crow = py.saturating_sub(capture.y0) * capture.w;
            for px in x_lo..x_hi {
                // A union member's composite coverage is its antialiased
                // ownership weight times the AA coverage of
                // `field < outer`, replacing the member's clip coverage
                // (the item's clip then carries the ancestors only);
                // the field is also what SDF effects read.
                let mut cc = clip_cov(clip, self.w, px, py);
                if cc <= 0.0 {
                    continue;
                }
                let field = if needs_field {
                    let (field, coverage) = sample_field(effect, union, px, py);
                    cc *= coverage;
                    if cc <= 0.0 {
                        continue;
                    }
                    field
                } else {
                    (0.0, 0.0, 0.0)
                };
                let c = match effect {
                    SampleEffect::None => pixel_sample(capture, px, py, crow),
                    SampleEffect::Color(m) => color_matrix(m, pixel_sample(capture, px, py, crow)),
                    SampleEffect::Sdf(sdf) => sdf_sample(capture, sdf, field, px, py, crow),
                };
                // The sample at its coverage blends source-over in the
                // group's member space: the canvas converts into it and
                // the result back.
                let src = move_space(simd, c.map(|v| v * cc), capture.space, member_space);
                let d = &mut dst[row + px];
                *d = if member_space == space {
                    src_over(*d, src)
                } else {
                    move_space(
                        simd,
                        src_over(move_space(simd, *d, space, member_space), src),
                        member_space,
                        space,
                    )
                };
            }
        }
    }

    /// Rasterizes one glyph mask instance: `mask` rows intersecting the
    /// band composite `paint * mask * clipcov`.
    #[expect(
        clippy::cast_possible_wrap,
        clippy::cast_possible_truncation,
        reason = "mask coordinates and pixel indices are small"
    )]
    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level so mul_add is one instruction"
    )]
    #[expect(
        clippy::too_many_arguments,
        reason = "the glyph context travels together"
    )]
    #[inline(always)]
    fn glyph<S: Simd>(
        &mut self,
        simd: S,
        stack: &mut Vec<Plane>,
        slot: &std::sync::OnceLock<std::sync::Arc<crate::render::glyph::GlyphMask>>,
        ox: i32,
        oy: i32,
        paint: &PaintData,
        clip: Option<&ClipCoverage<'_>>,
    ) {
        let Some(mask) = slot.get() else { return };
        let bh = self.fb.len() / self.w;
        let (mx0, my0) = (ox + mask.left, oy + mask.top);
        let (x_lo, x_hi) = (
            usize::try_from(mx0).unwrap_or(0).min(self.w),
            usize::try_from(mx0 + mask.w as i32)
                .unwrap_or(0)
                .min(self.w),
        );
        let (y_lo, y_hi) = (
            usize::try_from(my0)
                .unwrap_or(0)
                .saturating_sub(self.y0)
                .min(bh),
            usize::try_from(my0 + mask.h as i32)
                .unwrap_or(0)
                .saturating_sub(self.y0)
                .min(bh),
        );
        if y_lo >= y_hi || x_lo >= x_hi {
            return;
        }
        let (dst, space) = top(&mut *self.fb, self.space, stack.as_mut_slice());
        let mut src = [const { MaybeUninit::<[f32; 4]>::uninit() }; SPAN_CHUNK];
        for y in y_lo..y_hi {
            let py = self.y0 + y;
            let row = usize::try_from(py as i32 - my0).unwrap_or(0) * mask.w as usize;
            // The mask columns under `x_lo..x_hi`, indexed `x - mx0`.
            let m_lo = usize::try_from(x_lo as i32 - mx0).unwrap_or(0);
            let cov = &mask.cov[row + m_lo..row + m_lo + (x_hi - x_lo)];
            let mut start = 0;
            while start < cov.len() {
                while start < cov.len() && cov[start] <= 0.0 {
                    start += 1;
                }
                let mut end = start;
                while end < cov.len() && cov[end] > 0.0 {
                    end += 1;
                }
                if start < end {
                    shade_run(
                        simd,
                        &mut dst[y * self.w..(y + 1) * self.w],
                        Run::Slice(&cov[start..end]),
                        x_lo + start..x_lo + end,
                        py,
                        self.w,
                        paint,
                        clip,
                        space,
                        &mut src,
                    );
                }
                start = end;
            }
        }
    }

    /// Composites a projected local image over its parent-raster bounds.
    /// Each scanline starts the inverse homography's numerators and
    /// denominator at its first pixel centre and steps them across x; only
    /// the division is per sample.
    #[expect(
        clippy::cast_precision_loss,
        reason = "pixel indices are far below 2^53"
    )]
    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level so mul_add is one instruction"
    )]
    #[inline(always)]
    fn project<S: Simd>(
        &mut self,
        simd: S,
        stack: &mut [Plane],
        item: &super::lower::ProjectItem,
        clip: Option<ClipCoverage<'_>>,
    ) {
        let bh = self.fb.len() / self.w;
        let [x0, y0, x1, y1] = item.placed.bounds.map(|v| v as usize);
        let (x1, y_lo, y_hi) = (x1.min(self.w), y0.max(self.y0), y1.min(self.y0 + bh));
        let inverse = &item.placed.inverse;
        let step = [inverse.0[0][0], inverse.0[1][0], inverse.0[2][0]];
        for py in y_lo..y_hi {
            let mut q = inverse.map(x0 as f64 + 0.5, py as f64 + 0.5);
            for px in x0..x1 {
                let here = q;
                q = [q[0] + step[0], q[1] + step[1], q[2] + step[2]];
                let cc = clip_cov(clip, self.w, px, py);
                if cc <= 0.0 {
                    continue;
                }
                let sample = item.placed.image.sample(inverse, here);
                let src = sample.map(|v| v * item.opacity * cc);
                let (dst, space) = top(&mut *self.fb, self.space, stack);
                let i = (py - self.y0) * self.w + px;
                dst[i] = src_over(dst[i], member(simd, src, space));
            }
        }
    }

    /// Composites the popped isolation buffer onto the buffer below:
    /// the blend runs in the popped level's storage space (`src_space`,
    /// paired with its buffer), which is its declared `blend_space` for a
    /// semantic level or the parent's for a pass-through one — members
    /// already composited in it.
    #[expect(
        clippy::inline_always,
        reason = "compiled under the dispatched CPU level so mul_add is one instruction"
    )]
    #[inline(always)]
    fn composite_isolate<S: Simd>(
        &mut self,
        simd: S,
        (scratch, src_space): (&[[f32; 4]], cherenkov::BlendSpace),
        opacity: f32,
        blend: (cherenkov::BlendMode, cherenkov::BlendSpace),
        clip: Option<ClipCoverage<'_>>,
        stack: &mut Vec<Plane>,
    ) {
        let (dst, dst_space) = top(&mut *self.fb, self.space, stack.as_mut_slice());
        let direct = blend.0 == cherenkov::BlendMode::Normal && src_space == dst_space;
        if !direct && !super::blend::is_destructive(blend.0) {
            // Every other operator leaves the destination unchanged for a
            // transparent source, so the clip coverage scales the source
            // instead of bounding the whole composite. Only the pixels
            // under a non-empty source take the result. In a different
            // blend space the destination converts into `blended` a span
            // at a time and back; in its own space it blends in place.
            const SPAN: usize = 64;
            let mut source = [[0.0_f32; 4]; SPAN];
            let mut blended = [[0.0_f32; 4]; SPAN];
            for (row, (src_row, dst_row)) in scratch
                .chunks_exact(self.w)
                .zip(dst.chunks_exact_mut(self.w))
                .enumerate()
            {
                let py = self.y0 + row;
                for x0 in (0..self.w).step_by(SPAN) {
                    let n = SPAN.min(self.w - x0);
                    let mut any = false;
                    for (j, s) in source[..n].iter_mut().enumerate() {
                        let cc = clip_cov(clip, self.w, x0 + j, py);
                        *s = if cc > 0.0 {
                            src_row[x0 + j].map(|v| v * opacity * cc)
                        } else {
                            [0.0; 4]
                        };
                        any |= s.iter().any(|&channel| channel != 0.0);
                    }
                    if !any {
                        continue;
                    }
                    let (span, source) = (&mut dst_row[x0..x0 + n], &source[..n]);
                    if src_space == dst_space {
                        for (d, s) in span.iter_mut().zip(source) {
                            if s.iter().any(|&channel| channel != 0.0) {
                                *d = super::blend::blend(blend.0, *d, *s);
                            }
                        }
                        continue;
                    }
                    let blended = &mut blended[..n];
                    convert_span_into(simd, span, blended, src_space);
                    for (b, s) in blended.iter_mut().zip(source) {
                        *b = super::blend::blend(blend.0, *b, *s);
                    }
                    convert_span(simd, blended, src_space, dst_space);
                    for ((d, b), s) in span.iter_mut().zip(&*blended).zip(source) {
                        if s.iter().any(|&channel| channel != 0.0) {
                            *d = *b;
                        }
                    }
                }
            }
            return;
        }
        for (i, &src) in scratch.iter().enumerate() {
            let px = i % self.w;
            let py = self.y0 + i / self.w;
            let cc = clip_cov(clip, self.w, px, py);
            if direct {
                let s = src.map(|v| v * opacity * cc);
                dst[i] = src_over(dst[i], s);
            } else if cc > 0.0 {
                // The clip limits the composite operation, including Clear
                // and DestIn; multiplying only source alpha is not equivalent.
                let source = src.map(|value| value * opacity);
                let result = composite(
                    simd, blend.0, src_space, dst_space, src_space, dst[i], source,
                );
                dst[i] = if cc >= 1.0 {
                    result
                } else {
                    std::array::from_fn(|channel| {
                        cc.mul_add(result[channel] - dst[i][channel], dst[i][channel])
                    })
                };
            }
        }
    }
}

/// Rasterizes a full-surface mask with the same sparse compiler as draws.
/// The dense output is retained only by the mask's consumer.
pub fn coverage_mask(edges: &[Edge], rule: FillRule, w: usize, h: usize) -> Vec<f32> {
    if w == 0 || h == 0 {
        return Vec::new();
    }
    if cherenkov::lowering::proves_disjoint(edges) {
        let raw: Vec<_> = edges
            .iter()
            .copied()
            .map(super::tiles::IntegerEdge::from)
            .collect();
        return boundary_mask(&raw, w, h);
    }
    let raw: Vec<_> = edges
        .iter()
        .copied()
        .map(super::tiles::IntegerEdge::from)
        .collect();
    let edges = super::topology::Sweep::default().boundaries(&raw, rule);
    boundary_mask(edges.as_slice(), w, h)
}

/// A disjoint proof or topology compilation supplies the visible boundaries.
pub fn boundary_mask(edges: &[super::tiles::IntegerEdge], w: usize, h: usize) -> Vec<f32> {
    let edges = EdgeTiles::from_boundaries(edges);
    let mut mask = vec![0.0; w * h];
    mask.par_chunks_mut(BAND_H * w).enumerate().for_each_init(
        Compiler::default,
        |compiler, (band, slice)| {
            let y0 = band * BAND_H;
            let rows = slice.len() / w;
            compiler.prepare(&edges, super::tiles::Fixed::default(), y0..y0 + rows, w);
            compiler.rows(rows, |y, range, run| {
                for (i, x) in range.enumerate() {
                    slice[y * w + x] = run.at(i);
                }
            });
        },
    );
    mask
}

#[cfg(test)]
mod tests {
    use cherenkov_oracle::metrics::assert_close;

    use super::store_f16;

    /// `store_f16` must produce the same bits as the per-element
    /// `f16::from_f32` it replaces: the corpus stays bit-identical only
    /// if the slice conversion rounds identically in every boundary
    /// class — round-half-to-even ties, subnormals and their rounding
    /// points, the largest finite f16 and the first value that overflows
    /// to infinity, signed zeros, infinities and NaN.
    #[test]
    fn store_f16_matches_per_element_from_f32() {
        let src = [
            // Ties at the f16 mantissa rounding point go to even.
            [
                f32::from_bits(0x3F80_1000), // 1 + 2^-11: tie -> 1.0
                f32::from_bits(0x3F80_3000), // 1 + 3*2^-11: tie -> 1 + 2^-9
                f32::from_bits(0x3F80_1001), // just above a tie -> up
                f32::from_bits(0xBF80_1000), // -(1 + 2^-11): tie -> -1.0
            ],
            // Subnormal values and their rounding boundaries.
            [
                f32::from_bits(0x3380_0000), // 2^-24, smallest subnormal
                f32::from_bits(0xB380_0000), // -2^-24
                f32::from_bits(0x3420_0000), // 2.5*2^-24: tie -> 2*2^-24
                f32::from_bits(0x3300_0000), // 2^-25: tie to even -> +0
            ],
            [
                f32::from_bits(0x2F80_0000), // well below a subnormal -> +0
                f32::from_bits(0x341F_FFFF), // just below the 2.5*2^-24 tie
                f32::from_bits(0x3F80_0000), // 1.0
                f32::from_bits(0xC020_0000), // -2.5
            ],
            // The largest finite f16 and the values around overflow.
            [
                f32::from_bits(0x477F_E000), // 65504, largest finite f16
                f32::from_bits(0x477F_EFFF), // just below overflow -> 65504
                f32::from_bits(0x477F_F000), // 65520 -> +inf
                f32::from_bits(0xC77F_F000), // -65520 -> -inf
            ],
            // Signed zeros, infinities and NaN.
            [
                f32::from_bits(0x0000_0000), // +0
                f32::from_bits(0x8000_0000), // -0
                f32::from_bits(0x7F80_0000), // +inf
                f32::from_bits(0xFF80_0000), // -inf
            ],
            [
                f32::from_bits(0x7FC0_0000), // quiet NaN
                f32::from_bits(0x7F80_0001), // NaN with payload
                f32::from_bits(0x3DCC_CCCD), // 0.1
                f32::from_bits(0x0000_0001), // smallest f32 -> +0
            ],
        ];
        let mut dst = [[half::f16::ZERO; 4]; 6];
        store_f16(&mut dst, &src);
        for (dst_px, src_px) in dst.iter().zip(&src) {
            assert_eq!(
                dst_px.map(half::f16::to_bits),
                src_px.map(half::f16::from_f32).map(half::f16::to_bits),
                "src {src_px:?}"
            );
        }
    }

    #[test]
    fn band_claims_partition_output_across_workers() {
        use super::{AtomicUsize, BandQueue, NonZeroUsize, Ordering};
        use rayon::prelude::*;

        let chunk_len = NonZeroUsize::new(7).unwrap();
        for workers in [1, 2, 8] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap();
            for len in [0usize, 1, 6, 7, 8, 127, 4097] {
                for mode in 0..3 {
                    let selected: Vec<_> = (0..len.div_ceil(chunk_len.get()))
                        .map(|index| match mode {
                            0 => true,
                            1 => index % 2 == 0,
                            _ => false,
                        })
                        .collect();
                    let mut output = vec![usize::MAX; len];
                    let claims: Vec<_> = (0..len.div_ceil(chunk_len.get()))
                        .map(|_| AtomicUsize::new(0))
                        .collect();
                    {
                        let mut region = cherenkov::damage::Region::new((
                            1,
                            u32::try_from(selected.len()).unwrap(),
                        ));
                        for (index, &dirty) in selected.iter().enumerate() {
                            if dirty {
                                region.add(0, u32::try_from(index).unwrap(), 1, 1);
                            }
                        }
                        let queue = BandQueue::new(&mut output, chunk_len, Some(&region));
                        pool.install(|| {
                            (0..workers).into_par_iter().for_each(|_| {
                                while let Some((index, band)) = queue.claim() {
                                    assert_eq!(claims[index].fetch_add(1, Ordering::Relaxed), 0);
                                    for (offset, value) in band.iter_mut().enumerate() {
                                        *value = index * chunk_len.get() + offset;
                                    }
                                }
                            });
                        });
                        assert!(queue.claim().is_none());
                        assert_eq!(
                            queue.next.load(Ordering::Relaxed),
                            selected.iter().filter(|dirty| **dirty).count()
                        );
                    }
                    assert!(
                        claims
                            .iter()
                            .enumerate()
                            .all(|(index, count)| count.load(Ordering::Relaxed)
                                == usize::from(selected[index]))
                    );
                    assert!(output.iter().enumerate().all(|(index, &value)| value
                        == if selected[index / chunk_len.get()] {
                            index
                        } else {
                            usize::MAX
                        }));
                }
            }
        }
    }

    use super::{
        ClipCoverage, ClipRun, Edge, composite_solid, composite_span, convert_span, coverage_mask,
        member, reduce_rows, src_over,
    };
    use crate::render::lower::IRect;
    use cherenkov::{BlendSpace, FillRule};

    /// The long edges of water-rs/waterui#2665 against the independent f64
    /// area of the same polygon: through one accumulator over the whole
    /// height, as glyph masks rasterize, and through the 16-row bands of
    /// a coverage mask, in both directions and under both fill rules.
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
                let mut reference = cherenkov_oracle::coverage::Coverage::new(1, 1);
                let mut edges = Vec::new();
                for i in 0..vertices.len() {
                    let (mut a, mut b) = (vertices[i], vertices[(i + 1) % vertices.len()]);
                    if reverse {
                        std::mem::swap(&mut a, &mut b);
                    }
                    edges.push(Edge {
                        x0: a.0,
                        y0: a.1,
                        x1: b.0,
                        y1: b.1,
                    });
                    reference.add_line(
                        f64::from(a.0) - f64::from(pixel.0),
                        f64::from(a.1) - f64::from(pixel.1),
                        f64::from(b.0) - f64::from(pixel.0),
                        f64::from(b.1) - f64::from(pixel.1),
                    );
                }
                let expected = reference.finish(cherenkov_scene::FillRule::NonZero)[0];
                for rule in [FillRule::NonZero, FillRule::EvenOdd] {
                    let banded = coverage_mask(&edges, rule, size, size)[py * size + px];
                    {
                        let (route, actual) = ("banded", banded);
                        let actual = f64::from(actual);
                        assert!(
                            (actual - expected).abs() < 0.0002,
                            "{route} {rule:?} reverse={reverse} ({px},{py}): {actual} versus {expected}"
                        );
                    }
                }
            }
        }
    }

    /// Texel `(x, y)` of the source level: every channel varies, alpha
    /// included.
    fn texel(x: u8, y: u8) -> [f32; 4] {
        [
            f32::from(x),
            f32::from(y).mul_add(0.5, 0.25),
            f32::from(x * y),
            f32::from(x + 3 * y).mul_add(0.05, 0.1),
        ]
    }

    /// The mean of the listed source texels.
    fn mean(texels: &[(u8, u8)]) -> [f32; 4] {
        let n = f32::from(u8::try_from(texels.len()).expect("a box holds at most 4 texels"));
        std::array::from_fn(|c| texels.iter().map(|&(x, y)| texel(x, y)[c]).sum::<f32>() / n)
    }

    fn assert_texel(actual: [f32; 4], expected: [f32; 4]) {
        for (a, e) in actual.iter().zip(expected) {
            assert!(
                (a - e).abs() <= 1e-6,
                "texel {actual:?}, expected {expected:?}"
            );
        }
    }

    #[test]
    fn pyramid_partial_boxes_average_the_texels_present() {
        // A 5×5 source level whose rows 2..5 are kept: level rows 1..3,
        // three texels wide.
        let src: Vec<[f32; 4]> = (2..5)
            .flat_map(|y| (0..5).map(move |x| texel(x, y)))
            .collect();
        let mut dst = vec![[0.0; 4]; 2 * 3];
        reduce_rows(&src, 5, (2, 5), (1, 3), &mut dst);
        // A full box.
        assert_texel(dst[0], mean(&[(0, 2), (1, 2), (0, 3), (1, 3)]));
        // The right edge's partial box: column 4 alone, two rows.
        assert_texel(dst[2], mean(&[(4, 2), (4, 3)]));
        // The bottom edge's partial box: row 4 alone, two columns.
        assert_texel(dst[4], mean(&[(2, 4), (3, 4)]));
        // The corner: the single texel present.
        assert_texel(dst[5], texel(4, 4));
    }

    /// `composite_span` and `composite_solid` produce the scalar
    /// per-pixel expression — `(src * cov) * cc`, `member`,
    /// `src + dst * (1 − src.a)`, `cc <= 0` untouched — over run lengths
    /// 0–67 and start offsets 0–7, both storage spaces, no clip / partial
    /// rect clip / coverage clip / full rect clip, mixed and all-full
    /// coverage rows, and opaque, translucent, extended-range,
    /// transparent and signed-zero sources, under the detected level and
    /// the fallback token, to within `assert_close`.
    #[expect(
        clippy::too_many_lines,
        reason = "one matrix over spaces, clips and run shapes"
    )]
    #[test]
    fn span_kernels_match_scalar_expression() {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_precision_loss,
            clippy::cast_possible_wrap,
            reason = "test pattern indices and coverage fractions are small"
        )]
        #[expect(
            clippy::too_many_lines,
            reason = "one matrix over spaces, clips and run shapes"
        )]
        fn check<S: fearless_simd::Simd>(simd: S) {
            let colors: &[[f32; 4]] = &[
                [0.2, 0.5, 0.9, 1.0],   // opaque
                [0.2, 0.5, 0.9, 0.6],   // translucent
                [1.4, -0.3, 0.9, 1.2],  // extended range
                [0.0, 0.0, 0.0, 0.0],   // transparent
                [-0.0, 0.0, -0.0, 1.0], // signed zero
            ];
            for space in [BlendSpace::Linear, BlendSpace::SrgbEncoded] {
                for clip_kind in 0..4u8 {
                    for cov_kind in 0..2u8 {
                        for &color in colors {
                            for x0 in 0..=7usize {
                                for len in 0..=67usize {
                                    let w = x0 + len + 8;
                                    let py = 1usize;
                                    let mut cells = vec![0.0; w];
                                    for (i, cell) in cells[x0..x0 + len].iter_mut().enumerate() {
                                        *cell = match (i + x0) % 4 {
                                            0 => 0.25,
                                            1 => 1.0,
                                            2 => 0.0,
                                            _ => 0.5,
                                        };
                                    }
                                    let mask = match clip_kind {
                                        1 => Some(ClipCoverage::Rect(IRect {
                                            x0: (x0 + 2) as i32,
                                            y0: 0,
                                            x1: (x0 + 5) as i32,
                                            y1: 4,
                                        })),
                                        2 => Some(ClipCoverage::Samples {
                                            cells: &cells,
                                            top: py,
                                        }),
                                        3 => Some(ClipCoverage::Rect(IRect {
                                            x0: 0,
                                            y0: 0,
                                            x1: (x0 + len) as i32,
                                            y1: 4,
                                        })),
                                        _ => None,
                                    };
                                    let clip = ClipRun::new(mask.as_ref(), w, x0, py);
                                    let cov: Vec<f32> = if cov_kind == 1 {
                                        vec![1.0; len]
                                    } else {
                                        (0..len)
                                            .map(|i| match (i + x0) % 7 {
                                                0 => 0.0,
                                                1 | 3 | 5 => 1.0,
                                                2 => 0.35,
                                                4 => 0.5,
                                                _ => 0.75,
                                            })
                                            .collect()
                                    };
                                    let dst0: Vec<[f32; 4]> = (0..len)
                                        .map(|i| {
                                            [
                                                (i % 5) as f32 / 10.0,
                                                if i == 1 { -0.0 } else { (i % 3) as f32 / 10.0 },
                                                (i % 7) as f32 / 10.0,
                                                0.5,
                                            ]
                                        })
                                        .collect();
                                    let expect = |src: &[[f32; 4]]| -> Vec<[f32; 4]> {
                                        (0..len)
                                            .map(|i| {
                                                let cc = clip.at(i);
                                                if cc <= 0.0 {
                                                    return dst0[i];
                                                }
                                                src_over(
                                                    dst0[i],
                                                    member(
                                                        simd,
                                                        src[i].map(|v| v * cov[i] * cc),
                                                        space,
                                                    ),
                                                )
                                            })
                                            .collect()
                                    };
                                    let mut dst = dst0.clone();
                                    composite_solid(
                                        simd,
                                        &mut dst,
                                        cov.as_slice(),
                                        clip,
                                        color,
                                        space,
                                    );
                                    let expected = expect(&vec![color; len]);
                                    for (actual, want) in dst.iter().zip(&expected) {
                                        assert_close(actual, want);
                                    }
                                    let src: Vec<[f32; 4]> = (0..len)
                                        .map(|i| {
                                            color.map(|v| {
                                                v * (0.25 + 0.75 * ((i * 5 + x0) % 9) as f32 / 8.0)
                                            })
                                        })
                                        .collect();
                                    let mut dst = dst0.clone();
                                    composite_span(
                                        simd,
                                        &mut dst,
                                        &src,
                                        cov.as_slice(),
                                        clip,
                                        space,
                                    );
                                    let expected = expect(&src);
                                    for (actual, want) in dst.iter().zip(&expected) {
                                        assert_close(actual, want);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        cherenkov_simd::dispatch!(cherenkov_simd::detect(), simd => check(simd));
        check(fearless_simd::Fallback::new());
    }

    /// `convert_span` is filtrate-core's scalar `to_srgb`/`from_srgb` per
    /// pixel, to within `assert_close`,
    /// in both directions and as the identity, over run lengths 0–67 and every
    /// start offset, on finite, negative, HDR, transparent and
    /// signed-zero values, under the detected level and the fallback
    /// token.
    #[test]
    fn convert_span_matches_filtrate() {
        fn reference(pixel: [f32; 4], from: BlendSpace, to: BlendSpace) -> [f32; 4] {
            let mut pixel = [pixel];
            if from != to {
                if to == BlendSpace::SrgbEncoded {
                    filtrate_core::space::to_srgb(&mut pixel);
                } else {
                    filtrate_core::space::from_srgb(&mut pixel);
                }
            }
            pixel[0]
        }
        fn check<S: fearless_simd::Simd>(simd: S) {
            let seeds: &[[f32; 4]] = &[
                [0.2, 0.4, 0.6, 0.8],
                [1.4, -0.3, 0.9, 1.2],
                [0.0, 0.0, 0.0, 0.0],
                [-0.0, 0.0, -0.0, 1.0],
                [0.05, 0.15, 0.25, 0.35],
            ];
            let spaces = [
                (BlendSpace::Linear, BlendSpace::SrgbEncoded),
                (BlendSpace::SrgbEncoded, BlendSpace::Linear),
                (BlendSpace::Linear, BlendSpace::Linear),
                (BlendSpace::SrgbEncoded, BlendSpace::SrgbEncoded),
            ];
            for (from, to) in spaces {
                for &pixel in seeds {
                    let mut out = [pixel; 17];
                    convert_span(simd, &mut out, from, to);
                    let expected = reference(pixel, from, to);
                    for actual in out {
                        assert_close(&actual, &expected);
                    }
                }
                for x0 in 0..=7usize {
                    for len in 0..=67usize {
                        let pixels: Vec<[f32; 4]> =
                            (0..len).map(|i| seeds[(i + x0) % seeds.len()]).collect();
                        let expected: Vec<[f32; 4]> =
                            pixels.iter().map(|&p| reference(p, from, to)).collect();
                        let mut out = pixels;
                        convert_span(simd, &mut out, from, to);
                        for (actual, want) in out.iter().zip(&expected) {
                            assert_close(actual, want);
                        }
                    }
                }
            }
        }
        cherenkov_simd::dispatch!(cherenkov_simd::detect(), simd => check(simd));
        check(fearless_simd::Fallback::new());
    }
}
