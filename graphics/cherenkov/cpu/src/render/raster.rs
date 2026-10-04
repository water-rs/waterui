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

use rayon::prelude::*;
use rustc_hash::FxHashMap;
use std::ops::Range;

use cherenkov::FillRule;

use crate::render::lower::{
    ClipMask, ClipRef, FrameFilter, IRect, Item, SampleEffect, SdfEffect, SdfKind,
};
use crate::render::paint::PaintData;
use crate::{Band as BandOut, BandPixels};
use cherenkov::OffscreenFormat;
use filtrate_core::{CpuFilterError, CpuImage, WorkingSpace};

/// Rows per rasterization band.
pub const BAND_H: usize = 16;

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

/// The rows one group's captured backdrop holds this band.
struct Capture {
    /// `w × rows` premultiplied pixels starting at `y0`.
    buf: Vec<[f32; 4]>,
    /// First kept row's device y.
    y0: usize,
    /// Kept row count.
    rows: usize,
    /// First column (the capture region's `x0`).
    x0: usize,
    /// Row stride (the capture region's width).
    w: usize,
    /// The space the captured rows are stored in.
    space: cherenkov::BlendSpace,
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

/// Evaluates an [`SdfEffect`] at pixel centre `(px + 0.5, py + 0.5)`:
/// `Refraction` displaced-bilinear reads the capture, `Rim` adds the
/// highlight to the sampled colour (alpha unchanged — gaining alpha
/// would cancel under src-over).
#[expect(
    clippy::cast_precision_loss,
    reason = "pixels stay well below f32's integer bound"
)]
fn sdf_sample(capture: &Capture, sdf: &SdfEffect, px: usize, py: usize, crow: usize) -> [f32; 4] {
    let (d, nx, ny) = sdf_at(&sdf.edges, px as f32 + 0.5, py as f32 + 0.5);
    match sdf.kind {
        SdfKind::Refraction { depth, strength } => {
            let t = d.mul_add(depth.recip(), 1.0).clamp(0.0, 1.0);
            let qx = (nx * strength * t).mul_add(-t, px as f32 + 0.5);
            let qy = (ny * strength * t).mul_add(-t, py as f32 + 0.5);
            capture_sample(capture, qx, qy)
        }
        SdfKind::Rim { width, color, gain } => {
            let t = d.mul_add(width.recip(), 1.0).clamp(0.0, 1.0);
            let k = color[3] * gain * t * t;
            let s = capture.buf[crow + px - capture.x0];
            [
                color[0].mul_add(k, s[0]),
                color[1].mul_add(k, s[1]),
                color[2].mul_add(k, s[2]),
                s[3],
            ]
        }
    }
}

/// Signed distance and unit outward normal of `(x, y)` to a closed edge
/// boundary: `d < 0` inside; the normal points away from the interior.
/// Points on the boundary get distance `1e-6` and a zero normal (no
/// displacement — the measure-zero set).
fn sdf_at(edges: &[Edge], x: f32, y: f32) -> (f32, f32, f32) {
    let mut dist = f32::MAX;
    let (mut qx, mut qy) = (0.0f32, 0.0f32);
    let mut inside = false;
    for e in edges {
        let dx = e.x1 - e.x0;
        let dy = e.y1 - e.y0;
        let len2 = dx.mul_add(dx, dy * dy);
        let t = if len2 > 0.0 {
            ((x - e.x0).mul_add(dx, (y - e.y0) * dy)).clamp(0.0, len2) / len2
        } else {
            0.0
        };
        let cx = dx.mul_add(t, e.x0);
        let cy = dy.mul_add(t, e.y0);
        let dd = (x - cx).hypot(y - cy);
        if dd < dist {
            dist = dd;
            qx = cx;
            qy = cy;
        }
        if (e.y0 > y) != (e.y1 > y) {
            let xi = e.x0 + (y - e.y0) * (e.x1 - e.x0) / (e.y1 - e.y0);
            if x < xi {
                inside = !inside;
            }
        }
    }
    let d = dist.max(1e-6);
    let (nx, ny) = ((x - qx) / d, (y - qy) / d);
    if inside { (-d, -nx, -ny) } else { (d, nx, ny) }
}

/// Bilinear sample of `capture` at device point `(x, y)`: texel centres
/// at integer + 0.5, clamped to the capture region — the GPU's
/// `backdrop_sample` convention.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "the coordinates are clamped into the capture first"
)]
fn capture_sample(capture: &Capture, x: f32, y: f32) -> [f32; 4] {
    let fx = (x - capture.x0 as f32 - 0.5).clamp(0.0, capture.w as f32 - 1.0);
    let fy = (y - capture.y0 as f32 - 0.5).clamp(0.0, capture.rows as f32 - 1.0);
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
    let mut captures: Vec<(usize, usize, usize)> = Vec::new();
    if has_backdrop {
        let mut i = 0;
        while i < items.len() {
            match &items[i] {
                Item::PushFilter { end, .. } => {
                    i = usize::try_from(*end).unwrap_or(items.len());
                    continue;
                }
                Item::Capture(capture) => {
                    let (kept0, kept1) = kept_rows(&capture.union, capture.reach, surface, h);
                    if kept0 < kept1 {
                        captures.push((i, kept0, kept1));
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
    let coverage = std::mem::take(&mut scratch.coverage);
    let result = if let Some(&(last, ..)) = captures.last() {
        shade_windowed(
            items,
            clear,
            slice,
            w,
            surface,
            h,
            (&captures, last),
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

/// The band's pass with top-level captures: `run` over an expanded
/// window up to the last capture, then over the band for the rest.
///
/// The window covers the band plus every capture's `kept ± apron` rows;
/// the capture canvas clamps to its region inside it. On success the
/// central rows copy back into `slice` and each live isolation buffer
/// crops to band size.
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
    (captures, last): (&[(usize, usize, usize)], usize),
    coverage: Vec<f32>,
    scratch: &mut Scratch,
) -> Result<(), cherenkov::RenderError> {
    let (y0, y1) = surface;
    let bh = y1 - y0;
    let mut win0 = y0;
    let mut win1 = y1;
    for &(item, kept0, kept1) in captures {
        let Item::Capture(capture) = &items[item] else {
            unreachable!("scanned item kind");
        };
        win0 = win0.min(kept0.saturating_sub(capture.apron));
        win1 = win1.max(kept1.saturating_add(capture.apron));
    }
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
                let (kept0, kept1) = kept_rows(&capture.union, capture.reach, ctx.surface, h);
                if kept0 < kept1 {
                    capture_band(
                        band,
                        stack.as_slice(),
                        buffers,
                        ctx,
                        capture.group,
                        capture.region,
                        capture.apron,
                        capture.filter.as_ref(),
                        capture.flatten,
                        (kept0, kept1),
                    )?;
                }
            }
            Item::Sample {
                group,
                bounds,
                clip,
                effect,
            } => {
                if let Some(capture) = ctx.captures.get(group) {
                    band.sample(
                        capture,
                        *bounds,
                        clip.as_ref(),
                        effect,
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

/// Runs one backdrop group's capture for this band: copies the run's
/// `win` rows of the nearest semantic level with the trailing `flatten`
/// clip-only levels composited raw over it, applies the group's chain,
/// and keeps the `kept` rows for this band's samples.
#[expect(
    clippy::too_many_arguments,
    reason = "the item's fields travel together"
)]
fn capture_band(
    band: &Band<'_>,
    stack: &[Plane],
    buffers: &mut Buffers,
    ctx: &mut FrameCtx<'_>,
    group: u64,
    region: IRect,
    apron: usize,
    filter: Option<&FrameFilter>,
    flatten: usize,
    kept: (usize, usize),
) -> Result<(), cherenkov::RenderError> {
    let (kept0, kept1) = kept;
    let w = band.w;
    let rx0 = usize::try_from(region.x0).unwrap_or(0);
    let rx1 = usize::try_from(region.x1).unwrap_or(0).min(w);
    let ry0 = usize::try_from(region.y0).unwrap_or(0);
    let ry1 = usize::try_from(region.y1).unwrap_or(0);
    let rw = rx1.saturating_sub(rx0);
    // The window the chain reads: `kept` plus `apron` rows, clamped to
    // the region.
    let win0 = kept0.saturating_sub(apron).max(ry0);
    let win1 = kept1.saturating_add(apron).min(ry1);
    let rows = win1.saturating_sub(win0);
    if rw == 0 || rows == 0 {
        return Ok(());
    }
    let bh = band.fb.len() / w;
    if win0 < band.y0 || win1 > band.y0 + bh {
        return Err(cherenkov::RenderError::Render(
            "backdrop capture reads outside the run window".into(),
        ));
    }
    if flatten > stack.len() {
        return Err(cherenkov::RenderError::Render(
            "backdrop capture flatten underflow".into(),
        ));
    }
    let mut canvas = buffers.take_color(rw * rows);
    let base = stack.len() - flatten;
    // The flattened clip-only levels share the semantic level's storage
    // space, so the capture copies and composites in it raw.
    let space = if base == 0 {
        band.space
    } else {
        stack[base - 1].space
    };
    {
        // The nearest semantic level: the framebuffer when every live
        // level is clip-only, else the level below the flattened ones.
        let src: &[[f32; 4]] = if base == 0 {
            band.fb
        } else {
            &stack[base - 1].buf
        };
        for row in win0..win1 {
            let dst = &mut canvas[(row - win0) * rw..(row - win0) * rw + rw];
            dst.copy_from_slice(&src[(row - band.y0) * w + rx0..(row - band.y0) * w + rx0 + rw]);
        }
    }
    // Clip-only levels composite raw src-over, matching the oracle's
    // `flattened`.
    for level in &stack[base..] {
        for row in win0..win1 {
            for px in 0..rw {
                let dst = &mut canvas[(row - win0) * rw + px];
                *dst = src_over(*dst, level.buf[(row - band.y0) * w + rx0 + px]);
            }
        }
    }
    if let Some(filter) = filter {
        apply_filter(filter, &mut canvas, win0 - ry0, (rw, ry1 - ry0))?;
    }
    let kept_rows = kept1 - kept0;
    let mut buf = buffers.take_color(rw * kept_rows);
    buf.copy_from_slice(&canvas[(kept0 - win0) * rw..(kept0 - win0) * rw + rw * kept_rows]);
    buffers.give_color(canvas);
    buffers.meter.captured = true;
    ctx.captures.insert(
        group,
        Capture {
            buf,
            y0: kept0,
            rows: kept_rows,
            x0: rx0,
            w: rw,
            space,
        },
    );
    Ok(())
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
                let (dst, space) = top(&mut *self.fb, self.space, stack.as_mut_slice());
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
    /// (`SampleEffect::None` reads the capture unchanged). Writes are
    /// `clip`-coverage-gated: nothing lands outside the member clip.
    fn sample(
        &mut self,
        capture: &Capture,
        bounds: IRect,
        clip: Option<&ClipRef>,
        effect: &SampleEffect,
        stack: &mut [Plane],
    ) {
        let bh = self.fb.len() / self.w;
        let y_lo = usize::try_from(bounds.y0)
            .unwrap_or(0)
            .max(self.y0)
            .max(capture.y0);
        let y_hi = usize::try_from(bounds.y1)
            .unwrap_or(0)
            .min(self.y0 + bh)
            .min(capture.y0 + capture.rows);
        let x_lo = usize::try_from(bounds.x0)
            .unwrap_or(0)
            .max(capture.x0)
            .min(self.w);
        let x_hi = usize::try_from(bounds.x1)
            .unwrap_or(0)
            .min(capture.x0 + capture.w)
            .min(self.w);
        if y_lo >= y_hi || x_lo >= x_hi {
            return;
        }
        let (dst, space) = top(&mut *self.fb, self.space, stack);
        for py in y_lo..y_hi {
            let row = (py - self.y0) * self.w;
            let crow = (py - capture.y0) * capture.w;
            for px in x_lo..x_hi {
                let cc = clip_cov(clip, self.w, px, py);
                if cc <= 0.0 {
                    continue;
                }
                let c = match effect {
                    SampleEffect::None => capture.buf[crow + px - capture.x0],
                    SampleEffect::Color(m) => color_matrix(m, capture.buf[crow + px - capture.x0]),
                    SampleEffect::Sdf(sdf) => sdf_sample(capture, sdf, px, py, crow),
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
    /// parent's for a clip-only one — members already composited in it.
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
