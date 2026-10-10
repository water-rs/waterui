//! The banded exact-area coverage rasterizer.
//!
//! A port of the accumulation rasterizer from font-rs (`raster.rs`), also
//! used by `cherenkov-gpu` for glyph masks: every flattened directed edge
//! deposits a signed area into a `(width + 2) * band_h` accumulator whose
//! column 0 guards everything left of the canvas and whose last column
//! guards everything right of it, then each row is prefix-summed and the
//! fill rule turns the winding-weighted area into coverage.
//!
//! The accumulator is exact for polygons that do not self-overlap inside a
//! pixel — the oracle's `pixel_area` is exact even then; this is the known
//! difference this backend documents.

use arrayvec::ArrayVec;
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use std::ops::Range;

use cherenkov::FillRule;

use crate::render::lower::{
    BoxClip, CaptureItem, ClipMask, ClipRef, FrameFilter, IRect, Item, SampleEffect, SdfEffect,
    SdfKind, UnionSample,
};
use crate::render::paint::PaintData;
use crate::{Band as BandOut, BandPixels};
use cherenkov::OffscreenFormat;
use filtrate_core::{CpuFilterError, CpuImage, WorkingSpace};

/// Rows per rasterization band.
pub const BAND_H: usize = 16;

/// Texel rows a reduced capture keeps past the rows its sampled device
/// rows' bilinear taps reach, on each side: a tap that float rounding
/// moves across a texel boundary still lands on a kept row.
pub const SAMPLE_MARGIN: usize = 1;

/// One worker's reusable raster and filter buffers.
///
/// Created per rayon worker for the duration of a pass (`for_each_init`);
/// buffers are single-owner, never locked, and nothing is retained after the
/// pass.
///
/// The transient pixel bound is `workers × (band + Σ filter windows at the
/// deepest nesting)`, where each filter window is `w × (bh + 2·apron)`.
pub struct Scratch {
    /// `(w + 2) * band_rows` coverage cells, reused across bands.
    coverage: Vec<f32>,
    /// Isolation-stack colour buffers currently in use.
    stack: Vec<Plane>,
    buffers: Buffers,
    /// This band's captured backdrops, by group id.
    captures: Captures,
}

struct Buffers {
    free: Vec<Vec<[f32; 4]>>,
    coverage: Vec<Vec<f32>>,
    /// Live colour-buffer bytes and their peak for the current band.
    meter: Meter,
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

/// The rows capture `item` covers for the surface band `band` of a
/// surface `w × h`, `None` when the band samples none of it.
///
/// The device rows are the union's rows within the capture's reach of
/// the band. The item list's last capture, when its members displace
/// their reads, keeps only the rows its members' taps reach
/// ([`sample_rows`]); earlier captures keep every device row, because
/// later captures read the rows their members draw. A 1:1 capture keeps
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
fn capture_rows(
    item: &CaptureItem,
    items: &[Item],
    band: (usize, usize),
    (width, h): (usize, usize),
) -> Option<CaptureRows> {
    let (mut k0, mut k1) = kept_rows(&item.union, item.reach, band, h);
    let device = (k0, k1);
    if item.displaced
        && items.iter().rev().find_map(|item| match item {
            Item::Capture(capture) => Some(capture.group),
            _ => None,
        }) == Some(item.group)
    {
        let sampled = sample_rows(items, 0..items.len(), item.group, band, (width, h));
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
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "sample coordinates are finite and clamped to surface rows"
)]
fn sample_rows(
    items: &[Item],
    range: Range<usize>,
    group: u64,
    band: (usize, usize),
    (w, h): (usize, usize),
) -> (usize, usize) {
    let mut rows = (h, 0);
    let mut i = range.start;
    while i < range.end {
        match &items[i] {
            Item::PushFilter { end, apron, .. } => {
                let end = *end as usize;
                let nested = sample_rows(
                    items,
                    i + 1..end,
                    group,
                    (band.0.saturating_sub(*apron), (band.1 + apron).min(h)),
                    (w, h),
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
                        for py in first..last {
                            for px in x0..x1 {
                                if clip_cov(clip.as_ref(), w, px, py) <= 0.0 {
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
    rows
}

/// This band's captures by backdrop group id.
type Captures = FxHashMap<u64, Capture>;

/// Per-band state shared across the recursive `run` calls.
struct FrameCtx<'a> {
    /// The surface band rows `[y0, y1)` this run derives from.
    surface: (usize, usize),
    /// The surface height.
    h: usize,
    /// The band's captured backdrops.
    captures: &'a mut Captures,
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

impl Scratch {
    /// An empty working set; buffers grow to band size on first use.
    pub fn new() -> Self {
        Self {
            coverage: Vec::new(),
            stack: Vec::new(),
            buffers: Buffers {
                free: Vec::new(),
                coverage: Vec::new(),
                meter: Meter::default(),
            },
            captures: Captures::default(),
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

    fn take_coverage(&mut self, len: usize) -> Vec<f32> {
        let mut buf = self.coverage.pop().unwrap_or_default();
        buf.clear();
        buf.resize(len, 0.0);
        buf
    }

    fn give_coverage(&mut self, buf: Vec<f32>) {
        self.coverage.push(buf);
    }
}

/// Sample count of the shadow y-quadrature (the GPU uses the same).
const SHADOW_N: usize = 16;

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

/// A signed-area accumulation buffer over a band of `h` rows.
///
/// Columns are indexed `x + 1`, so column 0 collects every deposit at
/// `x <= -1` and column `w + 1` every deposit at `x >= w`. Deposits are
/// clamped into that range: their row sum is what the prefix sum consumes,
/// and a deposit's exact column below 0 or above `w` never changes the
/// coverage of an on-canvas pixel.
#[derive(Debug)]
pub struct Accum {
    w: usize,
    h: usize,
    /// `(w + 2) * h` cells.
    a: Vec<f32>,
    /// Guard-column window of the current draw: deposits clamp into
    /// `[cmin, cmax]` (inclusive); clearing and the prefix sum touch
    /// only that range.
    cmin: usize,
    cmax: usize,
}

impl Accum {
    /// A zeroed accumulator of `w` × `h` cells.
    pub fn new(w: usize, h: usize) -> Self {
        Self::with_buffer(w, h, Vec::new())
    }

    /// An accumulator of `w` × `h` cells on a reused zeroed buffer.
    pub fn with_buffer(w: usize, h: usize, mut a: Vec<f32>) -> Self {
        a.clear();
        a.resize((w + 2) * h, 0.0);
        Self {
            w,
            h,
            a,
            cmin: 0,
            cmax: w + 1,
        }
    }

    /// Releases the cell buffer for pooling.
    pub fn into_buffer(self) -> Vec<f32> {
        self.a
    }

    /// Restricts the draw window to pixels `x0..x1` (already clamped to
    /// `0..w`): deposits clamp into guard columns `x0..x1 + 1`, so a
    /// draw only ever pays its own bounding box's width.
    pub fn set_window(&mut self, x0: usize, x1: usize) {
        self.cmin = x0;
        self.cmax = (x1 + 1).min(self.w + 1);
    }

    /// Zeros the window's columns for band rows `y_lo..y_hi`.
    pub fn clear_range(&mut self, y_lo: usize, y_hi: usize) {
        for y in y_lo..y_hi.min(self.h) {
            let row = y * (self.w + 2);
            self.a[row + self.cmin..=row + self.cmax].fill(0.0);
        }
    }

    /// The accumulation cell for edge column `x` of band row `y`.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        clippy::cast_sign_loss,
        reason = "canvas width fits i32; the column is clamped non-negative"
    )]
    fn cell(&mut self, x: i32, y: usize) -> &mut f32 {
        let col = (x + 1).clamp(self.cmin as i32, self.cmax as i32) as usize;
        &mut self.a[y * (self.w + 2) + col]
    }

    /// Accumulates the signed area of the segment `(x0,y0)-(x1,y1)`, where
    /// `y` is band-local (`0..h`).
    #[expect(
        clippy::similar_names,
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::suboptimal_flops,
        reason = "a direct port of font-rs's scanline area accounting"
    )]
    pub fn draw_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) {
        if (y0 - y1).abs() <= f32::EPSILON {
            return;
        }
        let (dir, x0, y0, x1, y1) = if y0 < y1 {
            (1.0, x0, y0, x1, y1)
        } else {
            (-1.0, x1, y1, x0, y0)
        };
        let dxdy = (x1 - x0) / (y1 - y0);
        let mut x = x0;
        if y0 < 0.0 {
            x -= y0 * dxdy;
        }
        let y_start = y0.max(0.0) as usize;
        let y_end = self.h.min((y1.ceil() as usize).min(self.h));
        for y in y_start..y_end {
            let dy = ((y + 1) as f32).min(y1) - (y as f32).max(y0);
            let xnext = x + dxdy * dy;
            let d = dy * dir;
            let (xa, xb) = if x < xnext { (x, xnext) } else { (xnext, x) };
            let xa_floor = xa.floor();
            let xa_i = xa_floor as i32;
            let xb_ceil = xb.ceil();
            let xb_i = xb_ceil as i32;
            if xb_i <= xa_i + 1 {
                // The piece stays within one cell column.
                let xmf = x.midpoint(xnext) - xa_floor;
                *self.cell(xa_i, y) += d - d * xmf;
                *self.cell(xa_i + 1, y) += d * xmf;
            } else {
                let s = (xb - xa).recip();
                let xa_f = xa - xa_floor;
                let a0 = 0.5 * s * (1.0 - xa_f) * (1.0 - xa_f);
                let xb_f = xb - xb_ceil + 1.0;
                let am = 0.5 * s * xb_f * xb_f;
                *self.cell(xa_i, y) += d * a0;
                if xb_i == xa_i + 2 {
                    *self.cell(xa_i + 1, y) += d * (1.0 - a0 - am);
                } else {
                    let a1 = s * (1.5 - xa_f);
                    *self.cell(xa_i + 1, y) += d * (a1 - a0);
                    for xi in xa_i + 2..xb_i - 1 {
                        *self.cell(xi, y) += d * s;
                    }
                    let a2 = a1 + (xb_i - xa_i - 3) as f32 * s;
                    *self.cell(xb_i - 1, y) += d * (1.0 - a2 - am);
                    *self.cell(xb_i, y) += d * am;
                    x = xnext;
                    continue;
                }
                *self.cell(xb_i, y) += d * am;
            }
            x = xnext;
        }
    }

    /// Folds band-local row `y` into per-pixel coverage under `rule`,
    /// calling `f(x, coverage)` for each pixel `x0..x1`.
    ///
    /// The prefix sum starts at guard column `x0` and the coverage of
    /// pixel `x` is the sum through column `x + 1`.
    pub fn coverage_row(
        &self,
        y: usize,
        rule: FillRule,
        x0: usize,
        x1: usize,
        mut f: impl FnMut(usize, f32),
    ) {
        let row = y * (self.w + 2);
        let mut acc = self.a[row + x0];
        for x in x0..x1.min(self.w) {
            acc += self.a[row + x + 1];
            let cov = match rule {
                FillRule::NonZero => acc.abs().min(1.0),
                FillRule::EvenOdd => {
                    let m = acc.rem_euclid(2.0);
                    if m > 1.0 { 2.0 - m } else { m }
                }
            };
            if cov > 0.0 {
                f(x, cov);
            }
        }
    }
}

/// Abramowitz & Stegun 7.1.26 — the same approximation the GPU WGSL
/// uses, |error| < 1.5e-7.
#[expect(
    clippy::many_single_char_names,
    clippy::excessive_precision,
    clippy::suboptimal_flops,
    reason = "the A&S formula and its coefficients are cited verbatim"
)]
fn erf(x: f32) -> f32 {
    let s = x.signum();
    let a = x.abs();
    let t = (0.327_591_1_f32.mul_add(a, 1.0)).recip();
    let y = 1.0
        - (1.061_405_429_f32
            .mul_add(t, -1.453_152_027)
            .mul_add(t, 1.421_413_741)
            .mul_add(t, -0.284_496_736)
            .mul_add(t, 0.254_829_592))
            * t
            * (-a * a).exp();
    s * y
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

/// The clip coverage of `(px, py)`: 1/0 for the rect fast path, the mask
/// sample otherwise.
#[expect(
    clippy::cast_sign_loss,
    reason = "clip rect edges are clamped non-negative before indexing"
)]
fn clip_cov(clip: Option<&ClipRef>, w: usize, px: usize, py: usize) -> f32 {
    match clip.map(std::convert::AsRef::as_ref) {
        None => 1.0,
        Some(ClipMask::Rect(r)) => f32::from(
            px >= (r.x0.max(0) as usize)
                && px < (r.x1.max(0) as usize)
                && py >= (r.y0.max(0) as usize)
                && py < (r.y1.max(0) as usize),
        ),
        Some(ClipMask::Cover(mask)) => mask[py * w + px],
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
fn length(x: f32, y: f32) -> f32 {
    x.mul_add(x, y * y).sqrt()
}

/// The signed distance and unit outward normal of device point `(x, y)`
/// to `clip`'s boundary — the shader's `device_sdf` in f32: the point
/// mapped into box-local space by `inv`, the box's distance and outward
/// normal evaluated there, the normal mapped back by `inv`'s transpose
/// and normalised, and the distance divided by the same stretch.
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
/// storage (`convert_pixel` in `paint.rs`).
fn move_space(px: [f32; 4], from: cherenkov::BlendSpace, to: cherenkov::BlendSpace) -> [f32; 4] {
    if from == to {
        px
    } else {
        super::paint::convert_pixel(px, to == cherenkov::BlendSpace::SrgbEncoded)
    }
}

/// A member draw's linear premultiplied source in the level's storage
/// space: members composite with each other in the level's space.
fn member(px: [f32; 4], space: cherenkov::BlendSpace) -> [f32; 4] {
    move_space(px, cherenkov::BlendSpace::Linear, space)
}

/// Composite `src` onto `dst` blending in `space`: each operand converts
/// from its storage space, the blend applies, and the result lands back
/// in the destination's storage space.
fn composite(
    mode: cherenkov::BlendMode,
    space: cherenkov::BlendSpace,
    dst_space: cherenkov::BlendSpace,
    src_space: cherenkov::BlendSpace,
    dst: [f32; 4],
    src: [f32; 4],
) -> [f32; 4] {
    move_space(
        super::blend::blend(
            mode,
            move_space(dst, dst_space, space),
            move_space(src, src_space, space),
        ),
        space,
        dst_space,
    )
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
fn shade(
    items: &[Item],
    clear: [f32; 4],
    slice: &mut [[f32; 4]],
    w: usize,
    y0: usize,
    h: usize,
    scratch: &mut Scratch,
    peak: Option<&std::sync::atomic::AtomicU64>,
    has_backdrop: bool,
) -> Result<(), cherenkov::RenderError> {
    let bh = slice.len() / w;
    for (_, capture) in scratch.captures.drain() {
        scratch.buffers.give_color(capture.buf);
    }
    scratch.buffers.meter.reset();
    slice.fill(clear);
    let surface = (y0, y0 + bh);
    // The top-level captures this band serves (captures inside filter
    // scopes are handled by the scope's own windowed run). A surface that
    // used no backdrop group cannot contain a `Capture`, so the scan is
    // skipped outright.
    let mut last_capture = None;
    let mut window = surface;
    if has_backdrop {
        let mut i = 0;
        while i < items.len() {
            match &items[i] {
                Item::PushFilter { end, .. } => {
                    i = usize::try_from(*end).unwrap_or(items.len());
                    continue;
                }
                Item::Capture(capture) => {
                    if let Some(rows) = capture_rows(capture, items, surface, (w, h)) {
                        last_capture = Some(i);
                        window = (window.0.min(rows.window.0), window.1.max(rows.window.1));
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
    let coverage = std::mem::take(&mut scratch.coverage);
    let result = if let Some(last) = last_capture {
        shade_windowed(
            items,
            clear,
            slice,
            w,
            surface,
            h,
            (window, last),
            coverage,
            scratch,
        )
    } else {
        shade_plain(items, slice, w, surface, h, coverage, scratch)
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
    result
}

/// The band's pass with no capture: `run` over the caller's slice.
fn shade_plain(
    items: &[Item],
    slice: &mut [[f32; 4]],
    w: usize,
    surface: (usize, usize),
    h: usize,
    coverage: Vec<f32>,
    scratch: &mut Scratch,
) -> Result<(), cherenkov::RenderError> {
    let (y0, y1) = surface;
    let mut acc = Accum::with_buffer(w, y1 - y0, coverage);
    let mut band = Band {
        fb: slice,
        w,
        y0,
        space: cherenkov::BlendSpace::Linear,
    };
    let result = run(
        items,
        0..items.len(),
        &mut band,
        &mut acc,
        &mut scratch.stack,
        &mut scratch.buffers,
        &mut FrameCtx {
            surface,
            h,
            captures: &mut scratch.captures,
        },
    );
    scratch.coverage = acc.into_buffer();
    result
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
fn shade_windowed(
    items: &[Item],
    clear: [f32; 4],
    slice: &mut [[f32; 4]],
    w: usize,
    surface: (usize, usize),
    h: usize,
    ((win0, win1), last): ((usize, usize), usize),
    coverage: Vec<f32>,
    scratch: &mut Scratch,
) -> Result<(), cherenkov::RenderError> {
    let (y0, y1) = surface;
    let bh = y1 - y0;
    let win1 = win1.min(h);
    let rows = win1 - win0;
    let mut window = scratch.buffers.take_color(w * rows);
    window.fill(clear);
    let mut acc = Accum::with_buffer(w, rows, coverage);
    let mut band = Band {
        fb: &mut window,
        w,
        y0: win0,
        space: cherenkov::BlendSpace::Linear,
    };
    let first_pass = run(
        items,
        0..last + 1,
        &mut band,
        &mut acc,
        &mut scratch.stack,
        &mut scratch.buffers,
        &mut FrameCtx {
            surface,
            h,
            captures: &mut scratch.captures,
        },
    );
    if first_pass.is_ok() {
        // Collapse to the band: the central rows go to the output slice
        // and each live isolation buffer shrinks to band size.
        let first = (y0 - win0) * w;
        slice.copy_from_slice(&window[first..first + w * bh]);
        for plane in &mut scratch.stack {
            let mut cropped = scratch.buffers.take_color(w * bh);
            cropped.copy_from_slice(&plane.buf[first..first + w * bh]);
            scratch
                .buffers
                .give_color(std::mem::replace(&mut plane.buf, cropped));
        }
    }
    scratch.buffers.give_color(window);
    if let Err(error) = first_pass {
        scratch.coverage = acc.into_buffer();
        return Err(error);
    }
    let mut acc = Accum::with_buffer(w, bh, acc.into_buffer());
    let mut band = Band {
        fb: slice,
        w,
        y0,
        space: cherenkov::BlendSpace::Linear,
    };
    let result = run(
        items,
        last + 1..items.len(),
        &mut band,
        &mut acc,
        &mut scratch.stack,
        &mut scratch.buffers,
        &mut FrameCtx {
            surface,
            h,
            captures: &mut scratch.captures,
        },
    );
    scratch.coverage = acc.into_buffer();
    result
}

/// Rasterizes the whole surface's items into `fb`, parallel over bands.
///
/// Each worker reuses its coverage, isolation and filter-window buffers.
/// The transient pixel bound is `workers × (band + Σ filter windows at the
/// deepest nesting)`, with each window sized `w × (bh + 2·apron)`.
pub fn render_bands(
    items: &[Item],
    clear: [f32; 4],
    fb: &mut [[f32; 4]],
    w: usize,
    h: usize,
    peak: Option<&std::sync::atomic::AtomicU64>,
    has_backdrop: bool,
) -> Result<(u32, u32), cherenkov::RenderError> {
    let stats = stats(items);
    fb.par_chunks_mut(BAND_H * w)
        .enumerate()
        .try_for_each_init(Scratch::new, |scratch, (band, slice)| {
            shade(
                items,
                clear,
                slice,
                w,
                band * BAND_H,
                h,
                scratch,
                peak,
                has_backdrop,
            )
        })?;
    Ok(stats)
}

/// Rasterizes into `out`, converting each finished band to `f16`. Shading
/// runs at full `f32`; the store rounds once, at the output boundary.
pub fn render_bands_f16(
    items: &[Item],
    clear: [f32; 4],
    out: &mut [[half::f16; 4]],
    w: usize,
    h: usize,
    peak: Option<&std::sync::atomic::AtomicU64>,
    has_backdrop: bool,
) -> Result<(u32, u32), cherenkov::RenderError> {
    let stats = stats(items);
    out.par_chunks_mut(BAND_H * w)
        .enumerate()
        .try_for_each_init(Scratch::new, |scratch, (band, out_slice)| {
            let mut band_px = scratch.buffers.take_color(out_slice.len());
            let result = shade(
                items,
                clear,
                &mut band_px,
                w,
                band * BAND_H,
                h,
                scratch,
                peak,
                has_backdrop,
            );
            if result.is_ok() {
                for (dst, src) in out_slice.iter_mut().zip(band_px.iter()) {
                    *dst = src.map(half::f16::from_f32);
                }
            }
            scratch.buffers.give_color(band_px);
            result
        })?;
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
    items: &[Item],
    clear: [f32; 4],
    size: (usize, usize),
    emit: &mut Vec<[half::f16; 4]>,
    format: OffscreenFormat,
    sink: &mut dyn FnMut(BandOut<'_>),
    peak: Option<&std::sync::atomic::AtomicU64>,
    has_backdrop: bool,
) -> Result<(u32, u32), cherenkov::RenderError> {
    let stats = stats(items);
    let (w, h) = size;
    let mut scratch = Scratch::new();
    let mut band_px = scratch.buffers.take_color(w * h.min(BAND_H));
    let mut y0 = 0;
    while y0 < h {
        let bh = (h - y0).min(BAND_H);
        if let Err(error) = shade(
            items,
            clear,
            &mut band_px[..w * bh],
            w,
            y0,
            h,
            &mut scratch,
            peak,
            has_backdrop,
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
                emit.extend(
                    band_px[..w * bh]
                        .iter()
                        .map(|px| px.map(half::f16::from_f32)),
                );
                sink(BandOut {
                    y,
                    pixels: BandPixels::F16(emit.as_slice()),
                });
            }
        }
        y0 += bh;
    }
    scratch.buffers.give_color(band_px);
    Ok(stats)
}

const fn slice_len(w: usize, bh: usize) -> usize {
    w * bh
}

#[expect(
    clippy::too_many_lines,
    reason = "keeps ordered item processing and recursive filter scopes together"
)]
fn run(
    items: &[Item],
    range: Range<usize>,
    band: &mut Band<'_>,
    acc: &mut Accum,
    stack: &mut Vec<Plane>,
    buffers: &mut Buffers,
    ctx: &mut FrameCtx<'_>,
) -> Result<(), cherenkov::RenderError> {
    let bh = band.fb.len() / band.w;
    let h = ctx.h;
    let mut i = range.start;
    while i < range.end {
        match &items[i] {
            Item::Draw {
                edges,
                bbox,
                rule,
                paint,
                clip,
            } => band.draw(acc, stack, edges, *bbox, *rule, paint, clip.as_ref()),
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
                    i += 1;
                    continue;
                };
                band.composite_isolate(
                    &plane.buf,
                    plane.space,
                    *opacity,
                    *blend,
                    clip.as_ref(),
                    stack,
                );
                buffers.give_color(plane.buf);
            }
            Item::PushFilter { end, apron, space } => {
                let scope_end = usize::try_from(*end)
                    .map_err(|_| cherenkov::RenderError::Render("invalid filter scope".into()))?;
                if scope_end <= i || scope_end >= range.end {
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
                let coverage = buffers.take_coverage((band.w + 2) * (bottom - top));
                let nested_result = {
                    let mut filter_band = Band {
                        fb: &mut window,
                        w: band.w,
                        y0: top,
                        space: *space,
                    };
                    let mut filter_acc = Accum::with_buffer(band.w, bottom - top, coverage);
                    let mut filter_stack = Vec::new();
                    let result = run(
                        items,
                        i + 1..scope_end,
                        &mut filter_band,
                        &mut filter_acc,
                        &mut filter_stack,
                        buffers,
                        ctx,
                    );
                    for plane in filter_stack {
                        buffers.give_color(plane.buf);
                    }
                    buffers.give_coverage(filter_acc.into_buffer());
                    result
                };
                if let Err(error) = nested_result {
                    buffers.give_color(window);
                    return Err(error);
                }
                if let Err(error) = apply_filter(filter, &mut window, top, (band.w, h)) {
                    buffers.give_color(window);
                    return Err(error);
                }
                let first = (band.y0 - top) * band.w;
                let central = &window[first..first + band.fb.len()];
                band.composite_isolate(central, *space, *opacity, *blend, clip.as_ref(), stack);
                buffers.give_color(window);
                i = scope_end;
            }
            Item::PopFilter { .. } => {
                return Err(cherenkov::RenderError::Render(
                    "unpaired filter scope".into(),
                ));
            }
            Item::Capture(capture) => {
                if let Some(rows) = capture_rows(capture, items, ctx.surface, (band.w, h)) {
                    capture_band(band, stack.as_slice(), buffers, ctx, capture, rows)?;
                }
            }
            Item::Sample {
                group,
                bounds,
                clip,
                effect,
                union,
            } => {
                if let Some(capture) = ctx.captures.get(group) {
                    band.sample(
                        capture,
                        *bounds,
                        clip.as_ref(),
                        effect,
                        union.as_ref(),
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
            } => band.shadow(stack, rbox, radii, *sigma_eff, color, *bbox, clip.as_ref()),
            Item::Project(item) => band.project(stack, item),
            Item::Silhouette { slot, paint, clip } => {
                band.glyph(stack, slot, 0, 0, paint, clip.as_ref());
            }
            Item::Glyph {
                slot,
                x,
                y,
                paint,
                clip,
            } => band.glyph(stack, slot, *x, *y, paint, clip.as_ref()),
        }
        i += 1;
    }
    Ok(())
}

fn apply_filter(
    (filter, params): &FrameFilter,
    pixels: &mut [[f32; 4]],
    top: usize,
    size: (usize, usize),
) -> Result<(), cherenkov::RenderError> {
    filter
        .apply(
            params,
            &WorkingSpace::LINEAR_DISPLAY_P3,
            &mut CpuImage { pixels, top, size },
        )
        .map_err(|error| match error {
            CpuFilterError::GpuImage { .. } => {
                cherenkov::RenderError::Unsupported(crate::names::FILTER_GPU_IMAGE)
            }
        })
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
fn capture_band(
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
        apply_filter(filter, &mut canvas, win0 - ry0, (rw, ry1 - ry0))?;
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
    /// Rasterizes one draw item into the top isolation buffer.
    #[expect(
        clippy::cast_precision_loss,
        clippy::too_many_arguments,
        reason = "pixel indices and band offsets are far below 2^24"
    )]
    fn draw(
        &mut self,
        acc: &mut Accum,
        stack: &mut Vec<Plane>,
        edges: &[Edge],
        bbox: crate::render::lower::IRect,
        rule: FillRule,
        paint: &PaintData,
        clip: Option<&ClipRef>,
    ) {
        let bh = self.fb.len() / self.w;
        // Band-intersect the device-space bounding box.
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
        if y_lo >= y_hi {
            return;
        }
        let (x_lo, x_hi) = (
            usize::try_from(bbox.x0).unwrap_or(0).min(self.w),
            usize::try_from(bbox.x1).unwrap_or(0).min(self.w),
        );
        // Only the item's bbox columns participate: clear and deposit
        // inside the guard window `x_lo .. x_hi + 1`.
        acc.set_window(x_lo, x_hi);
        acc.clear_range(y_lo, y_hi);
        for e in edges {
            // Only edges crossing the band deposit anything.
            let ey0 = e.y0 - self.y0 as f32;
            let ey1 = e.y1 - self.y0 as f32;
            if ey0.max(ey1) < 0.0 || ey0.min(ey1) >= bh as f32 {
                continue;
            }
            acc.draw_line(e.x0, ey0, e.x1, ey1);
        }
        let (dst, space) = top(&mut *self.fb, self.space, stack.as_mut_slice());
        for y in y_lo..y_hi {
            let py = self.y0 + y;
            acc.coverage_row(y, rule, x_lo, x_hi, |x, cov| {
                let cc = clip_cov(clip, self.w, x, py);
                if cc <= 0.0 {
                    return;
                }
                let src = paint
                    .eval(x as f32 + 0.5, py as f32 + 0.5)
                    .map(|v| v * cov * cc);
                dst[y * self.w + x] = src_over(dst[y * self.w + x], member(src, space));
            });
        }
    }

    /// Rasterizes a blurred rounded box: analytic coverage per pixel in
    /// `bbox` ∩ band.
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::float_cmp,
        clippy::suboptimal_flops,
        clippy::too_many_arguments,
        clippy::too_many_lines,
        reason = "pixel indices and band offsets are far below 2^24; the \
                  flat-row test is intentionally exact and the quadrature \
                  weights mirror the reference formula"
    )]
    fn shadow(
        &mut self,
        stack: &mut Vec<Plane>,
        rbox: &[f32; 4],
        radii: &[f32; 4],
        sigma_eff: f32,
        color: &[f32; 4],
        bbox: IRect,
        clip: Option<&ClipRef>,
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
        let (cx_lo, cx_hi, clip_mask) = match clip.map(AsRef::as_ref) {
            Some(ClipMask::Rect(r)) => (
                usize::try_from(r.x0).unwrap_or(0).clamp(x_lo, x_hi),
                usize::try_from(r.x1).unwrap_or(0).clamp(x_lo, x_hi),
                None,
            ),
            Some(ClipMask::Cover(mask)) => (x_lo, x_hi, Some(mask.as_slice())),
            None => (x_lo, x_hi, None),
        };
        if cx_lo >= cx_hi && clip_mask.is_none() {
            return;
        }
        // The 16 y-quadrature samples and their `gaussian*step` weights
        // are per item, not per pixel.
        let step = 6.0 * sigma / SHADOW_N as f32;
        let mut dy = [0.0_f32; SHADOW_N];
        let mut gw = [0.0_f32; SHADOW_N];
        for (i, (d, w)) in dy.iter_mut().zip(gw.iter_mut()).enumerate() {
            *d = (i as f32 + 0.5) * step - 3.0 * sigma;
            *w = gaussian(*d, sigma) * step;
        }
        // Per row the quadrature's x-integral saturates except near the
        // left/right edges: `erf` reaches ±1 within `MARGIN` of an edge,
        // so interior columns all evaluate to the same `S` (the sum of
        // in-box sample weights) and only the two edge bands pay the
        // 32-`erf` sum. For `sigma`-wide margins the tail error is
        // `erf(5)-1 < 1e-11`.
        let margin = 5.0 * sigma * std::f32::consts::SQRT_2;
        // Flat rows — every sample has zero corner inset — share the
        // straight-wall x-integral, one `erf` pair per column computed
        // once for the whole item.
        let mut x_term: Option<Vec<f32>> = None;
        let mut xl = [0.0_f32; SHADOW_N];
        let mut xr = [0.0_f32; SHADOW_N];
        let mut wg = [0.0_f32; SHADOW_N];
        for y in y_lo..y_hi {
            let py_i = self.y0 + y;
            let py = py_i as f32 + 0.5 - cy;
            // The row's sample table: left/right x-edges per in-box
            // sample, their weights, the interior-coverage sum `s`, and
            // the most-inset edges bounding the interior zone.
            let mut n = 0_usize;
            let mut s = 0.0_f32;
            let mut flat = true;
            let (mut xlm, mut xrm) = (f32::MIN, f32::MAX);
            for (d, w) in dy.iter().zip(gw.iter()) {
                let yi = py + d;
                if yi.abs() > half[1] {
                    continue;
                }
                let (rl, rr) = if yi < 0.0 {
                    (radii[0], radii[1])
                } else {
                    (radii[3], radii[2])
                };
                let ay = yi.abs();
                xl[n] = -half[0] + corner_inset(rl, ay - (half[1] - rl));
                xr[n] = half[0] - corner_inset(rr, ay - (half[1] - rr));
                wg[n] = *w;
                flat &= xl[n] == -half[0] && xr[n] == half[0];
                xlm = xlm.max(xl[n]);
                xrm = xrm.min(xr[n]);
                s += w;
                n += 1;
            }
            if n == 0 || s <= 0.0 {
                continue;
            }
            let (dst, space) = top(&mut *self.fb, self.space, stack.as_mut_slice());
            if flat {
                // Separable: `cov = x_term[px] * s` across the row.
                let t = x_term.get_or_insert_with(|| {
                    (cx_lo..cx_hi)
                        .map(|x| {
                            let px = x as f32 + 0.5 - cx;
                            0.5 * (erf((half[0] - px) * inv_sqrt2_sigma)
                                - erf((-half[0] - px) * inv_sqrt2_sigma))
                        })
                        .collect()
                });
                for x in cx_lo..cx_hi {
                    let cov = t[x - cx_lo] * s;
                    if cov <= 0.0 {
                        continue;
                    }
                    let cc = clip_mask.map_or(1.0, |m| m[py_i * self.w + x]);
                    if cc <= 0.0 {
                        continue;
                    }
                    let src = color.map(|v| v * cov.clamp(0.0, 1.0) * cc);
                    dst[y * self.w + x] = src_over(dst[y * self.w + x], member(src, space));
                }
                continue;
            }
            // Interior columns: `px >= xlm + margin` saturates every
            // `erf((xl-px)*is)` at -1 and `px <= xrm - margin` saturates
            // every `erf((xr-px)*is)` at +1, so `cov = s` for all of them.
            let x_in_lo = usize::try_from((cx + xlm + margin - 0.5).ceil() as i32)
                .unwrap_or(0)
                .clamp(cx_lo, cx_hi);
            let x_in_hi = usize::try_from((cx + xrm - margin + 0.5).floor() as i32 + 1)
                .unwrap_or(0)
                .clamp(x_in_lo, cx_hi);
            let (dst, space) = top(&mut *self.fb, self.space, stack.as_mut_slice());
            let edge = |dst: &mut [[f32; 4]], f: usize, t: usize| {
                for x in f..t {
                    let px = x as f32 + 0.5 - cx;
                    let mut cov = 0.0_f32;
                    for k in 0..n {
                        cov += 0.5
                            * (erf((xr[k] - px) * inv_sqrt2_sigma)
                                - erf((xl[k] - px) * inv_sqrt2_sigma))
                            * wg[k];
                    }
                    if cov <= 0.0 {
                        continue;
                    }
                    let cc = clip_mask.map_or(1.0, |m| m[py_i * self.w + x]);
                    if cc <= 0.0 {
                        continue;
                    }
                    let src = color.map(|v| v * cov.clamp(0.0, 1.0) * cc);
                    dst[y * self.w + x] = src_over(dst[y * self.w + x], member(src, space));
                }
            };
            edge(&mut *dst, cx_lo, x_in_lo);
            for x in x_in_lo..x_in_hi {
                let cc = clip_mask.map_or(1.0, |m| m[py_i * self.w + x]);
                if cc <= 0.0 {
                    continue;
                }
                let src = color.map(|v| v * s.clamp(0.0, 1.0) * cc);
                dst[y * self.w + x] = src_over(dst[y * self.w + x], member(src, space));
            }
            edge(&mut *dst, x_in_hi, cx_hi);
        }
    }

    /// Composites `capture`'s rows over the band's top inside `bounds`,
    /// under `clip` — the member's backdrop sample.
    /// Samples `capture` under `clip` with the member's `effect`
    /// (`SampleEffect::None` reads the capture unchanged) at full
    /// strength — the member's own scope attenuates it at composite.
    /// Writes are `clip`-coverage-gated: nothing lands outside the
    /// member clip.
    fn sample(
        &mut self,
        capture: &Capture,
        bounds: IRect,
        clip: Option<&ClipRef>,
        effect: &SampleEffect,
        union: Option<&UnionSample>,
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
                let src = c.map(|v| v * cc);
                dst[row + px] = src_over(dst[row + px], move_space(src, capture.space, space));
            }
        }
    }

    /// Rasterizes one glyph mask instance: `mask` rows intersecting the
    /// band composite `paint * mask * clipcov`.
    #[expect(
        clippy::cast_possible_wrap,
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        reason = "mask coordinates and pixel indices are small"
    )]
    fn glyph(
        &mut self,
        stack: &mut Vec<Plane>,
        slot: &std::sync::OnceLock<std::sync::Arc<crate::render::glyph::GlyphMask>>,
        ox: i32,
        oy: i32,
        paint: &PaintData,
        clip: Option<&ClipRef>,
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
        for y in y_lo..y_hi {
            let py = self.y0 + y;
            let row = usize::try_from(py as i32 - my0).unwrap_or(0) * mask.w as usize;
            for x in x_lo..x_hi {
                let cc = clip_cov(clip, self.w, x, py);
                if cc <= 0.0 {
                    continue;
                }
                let cov = mask.cov[row + usize::try_from(x as i32 - mx0).unwrap_or(0)];
                if cov <= 0.0 {
                    continue;
                }
                let src = paint
                    .eval(x as f32 + 0.5, py as f32 + 0.5)
                    .map(|v| v * cov * cc);
                let (dst, space) = top(&mut *self.fb, self.space, stack.as_mut_slice());
                dst[y * self.w + x] = src_over(dst[y * self.w + x], member(src, space));
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
    fn project(&mut self, stack: &mut [Plane], item: &super::lower::ProjectItem) {
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
                let cc = clip_cov(item.clip.as_ref(), self.w, px, py);
                if cc <= 0.0 {
                    continue;
                }
                let sample = item.placed.image.sample(inverse, here);
                let src = sample.map(|v| v * item.opacity * cc);
                let (dst, space) = top(&mut *self.fb, self.space, stack);
                let i = (py - self.y0) * self.w + px;
                dst[i] = src_over(dst[i], member(src, space));
            }
        }
    }

    /// Composites the popped isolation buffer onto the buffer below:
    /// the blend runs in the popped level's storage space (`src_space`),
    /// which is its declared `blend_space` for a semantic level or the
    /// parent's for a pass-through one — members already composited in
    /// it.
    fn composite_isolate(
        &mut self,
        scratch: &[[f32; 4]],
        src_space: cherenkov::BlendSpace,
        opacity: f32,
        blend: (cherenkov::BlendMode, cherenkov::BlendSpace),
        clip: Option<&ClipRef>,
        stack: &mut Vec<Plane>,
    ) {
        let (dst, dst_space) = top(&mut *self.fb, self.space, stack.as_mut_slice());
        for (i, &src) in scratch.iter().enumerate() {
            let px = i % self.w;
            let py = self.y0 + i / self.w;
            let cc = clip_cov(clip, self.w, px, py);
            if blend.0 == cherenkov::BlendMode::Normal && src_space == dst_space {
                let s = src.map(|v| v * opacity * cc);
                dst[i] = src_over(dst[i], s);
            } else if super::blend::is_destructive(blend.0) {
                // The clip limits the composite operation, including Clear
                // and DestIn; multiplying only source alpha is not equivalent.
                if cc > 0.0 {
                    let source = src.map(|value| value * opacity);
                    let result =
                        composite(blend.0, src_space, dst_space, src_space, dst[i], source);
                    dst[i] = if cc >= 1.0 {
                        result
                    } else {
                        std::array::from_fn(|channel| {
                            cc.mul_add(result[channel] - dst[i][channel], dst[i][channel])
                        })
                    };
                }
            } else if cc > 0.0 {
                // Every other operator leaves the destination unchanged
                // for a transparent source, so the clip coverage scales the
                // source instead of bounding the whole composite.
                let s = src.map(|v| v * opacity * cc);
                dst[i] = composite(blend.0, src_space, dst_space, src_space, dst[i], s);
            }
        }
    }
}

/// Rasterizes a full-surface coverage mask of `edges` under `rule`,
/// parallel over bands.
#[expect(
    clippy::cast_precision_loss,
    reason = "band origins are small integers"
)]
pub fn coverage_mask(edges: &[Edge], rule: FillRule, w: usize, h: usize) -> Vec<f32> {
    let mut mask = vec![0.0; w * h];
    mask.par_chunks_mut(BAND_H * w)
        .enumerate()
        .for_each(|(band, slice)| {
            let y0 = band * BAND_H;
            let bh = slice.len() / w;
            let mut acc = Accum::new(w, bh);
            for e in edges {
                let ey0 = e.y0 - y0 as f32;
                let ey1 = e.y1 - y0 as f32;
                if ey0.max(ey1) < 0.0 || ey0.min(ey1) >= bh as f32 {
                    continue;
                }
                acc.draw_line(e.x0, ey0, e.x1, ey1);
            }
            for y in 0..bh {
                acc.coverage_row(y, rule, 0, w, |x, cov| {
                    slice[y * w + x] = cov;
                });
            }
        });
    mask
}

#[cfg(test)]
mod tests {
    use super::reduce_rows;

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
}
