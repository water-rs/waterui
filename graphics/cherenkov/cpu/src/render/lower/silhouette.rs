//! Coverage convolution for silhouettes without an analytic shadow form.
use super::{FLATTEN_TOL, IRect, Item, Lowering, coverage_mask, flatten_edges};
use crate::names;
use crate::render::raster::Edge;
use crate::render::{glyph::GlyphMask, paint::PaintData};
use cherenkov::lowering::shadow::{gaussian_taps, spread_taps};
use cherenkov::lowering::shape_outline;
use cherenkov::{RenderError, Shadow, ShapeData};
use kurbo::Affine;
use std::sync::{Arc, OnceLock};

impl Lowering<'_, '_> {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        reason = "validated finite extents and bounded surface coordinates"
    )]
    pub(super) fn silhouette(
        &mut self,
        shape: &ShapeData,
        shadow: &Shadow,
    ) -> Result<(), RenderError> {
        if !shadow.spread.is_finite() {
            return Err(RenderError::Render("non-finite shadow parameters".into()));
        }
        if !self.transform.is_finite() || !self.transform.inverse().is_finite() {
            return Err(RenderError::Render("invalid silhouette transform".into()));
        }
        if !shadow.offset.x.is_finite() || !shadow.offset.y.is_finite() {
            return Err(RenderError::Render("non-finite shadow offset".into()));
        }
        let sigma = shadow.sigma;
        let [ma, mb, mc, md, _, _] = self.transform.as_coeffs();
        let scale = ma.hypot(mb).max(mc.hypot(md)).max(1e-12);
        let Some((path, rule)) = shape_outline(shape, FLATTEN_TOL / scale) else {
            return Err(RenderError::Unsupported(names::SHADOW));
        };
        let mut edges = flatten_edges(
            self.transform * Affine::translate(shadow.offset) * path,
            FLATTEN_TOL,
        );
        // The passes in order: spread, then the Gaussian along each axis.
        let mut passes: [Option<(Vec<[f32; 3]>, u8)>; 3] = [None, None, None];
        if shadow.spread != 0.0 {
            let taps = spread_taps([ma, mb, mc, md], shadow.spread, f64::from(i32::MAX))?;
            passes[0] = Some((taps, if shadow.spread > 0.0 { 1 } else { 2 }));
        }
        if sigma > 0.0 {
            for (slot, axis) in passes[1..].iter_mut().zip([[ma, mb], [mc, md]]) {
                *slot = Some((gaussian_taps(axis, sigma, f64::from(i32::MAX))?, 0));
            }
        }
        let passes: Vec<(Vec<[f32; 3]>, u8)> = passes.into_iter().flatten().collect();
        let reaches: Vec<(i32, i32)> = passes.iter().map(|(taps, _)| reach(taps)).collect();
        let total = reaches.iter().fold((0_i64, 0_i64), |(x, y), &(rx, ry)| {
            (x + i64::from(rx), y + i64::from(ry))
        });
        let limit = i64::from(i32::MAX) / 4;
        if total.0 > limit || total.1 > limit {
            return Err(RenderError::Render(
                "shadow capture exceeds addressable extent".into(),
            ));
        }
        if edges.is_empty() {
            return Ok(());
        }
        let Some((visible, active)) = footprint(&edges, &reaches, (self.width, self.height)) else {
            return Ok(());
        };
        let passes_len = passes.len();
        for rect in &active[..=passes_len] {
            rect.width()
                .checked_mul(rect.height())
                .filter(|n| *n <= isize::MAX as usize / 4)
                .ok_or_else(|| RenderError::Render("shadow coverage overflow".into()))?;
        }
        let origin = active[0];
        for e in &mut edges {
            e.x0 -= origin.x0 as f32;
            e.x1 -= origin.x0 as f32;
            e.y0 -= origin.y0 as f32;
            e.y1 -= origin.y0 as f32;
        }
        // Every parallel pass installs on the renderer's pool: the
        // closures capture only the bound locals — `Send` inputs — never
        // `self` (its borrowed fields are not `Send`).
        let (pool, level) = (self.pool, self.level);
        let mut coverage =
            pool.install(|| coverage_mask(&edges, rule, origin.width(), origin.height()));
        let mut from = origin;
        for (k, (taps, mode)) in passes.iter().enumerate() {
            let to = active[k + 1];
            coverage = pool.install(|| convolve(level, &coverage, from, to, taps, *mode));
            from = to;
        }
        let slot = Arc::new(OnceLock::new());
        slot.set(Arc::new(GlyphMask {
            left: visible.x0,
            top: visible.y0,
            w: visible.width() as u32,
            h: visible.height() as u32,
            cov: coverage,
        }))
        .expect("new coverage slot");
        let [red, green, blue, alpha] = shadow.color.components;
        self.items.push(Item::Silhouette {
            slot,
            paint: PaintData::Solid([red * alpha, green * alpha, blue * alpha, alpha]),
            clip: self.clip.clone(),
        });
        Ok(())
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "sampling finite capture coordinates with explicit bounds checks"
)]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn sample(src: &[f32], size: (usize, usize), x: f32, y: f32) -> f32 {
    let (left, top) = (x.floor(), y.floor());
    let (fx, fy) = (x - left, y - top);
    let at = |x: f32, y: f32| {
        if x < 0.0 || y < 0.0 || x >= size.0 as f32 || y >= size.1 as f32 {
            0.0
        } else {
            src[y as usize * size.0 + x as usize]
        }
    };
    let a = (at(left + 1.0, top) - at(left, top)).mul_add(fx, at(left, top));
    let b = (at(left + 1.0, top + 1.0) - at(left, top + 1.0)).mul_add(fx, at(left, top + 1.0));
    (b - a).mul_add(fy, a)
}

/// The buffer rectangles of a silhouette chain over `edges` (device space)
/// whose passes reach `reaches` far: the retained mask, then the box of the
/// raster and of each pass output. `None` when nothing lands on the surface.
///
/// Each box is where its buffer can be non-zero, limited to what a later
/// pass or the mask reads, so the last one is the mask itself.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "bounds are clamped to the surface before they narrow"
)]
fn footprint(
    edges: &[Edge],
    reaches: &[(i32, i32)],
    surface: (usize, usize),
) -> Option<(IRect, [IRect; 4])> {
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for e in edges {
        for (x, y) in [(e.x0, e.y0), (e.x1, e.y1)] {
            lo = [lo[0].min(f64::from(x)), lo[1].min(f64::from(y))];
            hi = [hi[0].max(f64::from(x)), hi[1].max(f64::from(y))];
        }
    }
    // The shape's device bounds, one pixel wider than its edges.
    let bounds = [
        lo[0].floor() - 1.0,
        lo[1].floor() - 1.0,
        hi[0].ceil() + 1.0,
        hi[1].ceil() + 1.0,
    ];
    let total = reaches
        .iter()
        .fold((0, 0), |(x, y), &(rx, ry)| (x + rx, y + ry));
    let (rx, ry) = (f64::from(total.0), f64::from(total.1));
    let visible = IRect {
        x0: (bounds[0] - rx).max(0.0) as i32,
        y0: (bounds[1] - ry).max(0.0) as i32,
        x1: (bounds[2] + rx).min(surface.0 as f64) as i32,
        y1: (bounds[3] + ry).min(surface.1 as f64) as i32,
    };
    if visible.is_empty() {
        return None;
    }
    // Backward: what each pass must produce for the next to read.
    let mut needed = [visible; 4];
    for k in (0..reaches.len()).rev() {
        needed[k] = needed[k + 1].grow(reaches[k].0, reaches[k].1);
    }
    // Forward: where each buffer can be non-zero, limited to what is read.
    let reads = needed[0];
    let mut active = [visible; 4];
    active[0] = IRect {
        x0: bounds[0].max(f64::from(reads.x0)) as i32,
        y0: bounds[1].max(f64::from(reads.y0)) as i32,
        x1: bounds[2].min(f64::from(reads.x1)) as i32,
        y1: bounds[3].min(f64::from(reads.y1)) as i32,
    };
    let mut support = active[0];
    for (k, &(rx, ry)) in reaches.iter().enumerate() {
        support = support.grow(rx, ry);
        active[k + 1] = support.intersect(needed[k + 1]);
    }
    debug_assert_eq!(active[reaches.len()], visible);
    Some((visible, active))
}

/// The columns and rows a bilinear read at any of `taps` can touch beyond the
/// cell itself.
#[expect(clippy::cast_possible_truncation, reason = "tap offsets are bounded")]
fn reach(taps: &[[f32; 3]]) -> (i32, i32) {
    taps.iter().fold((0, 0), |(rx, ry), &[dx, dy, _]| {
        (
            rx.max(dx.abs().floor() as i32 + 1),
            ry.max(dy.abs().floor() as i32 + 1),
        )
    })
}

/// Produces the `to` box of a pass over `src`, which covers the `from` box.
/// Reads outside `from` are zero coverage.
#[expect(clippy::cast_precision_loss, reason = "capture coordinates fit f32")]
fn convolve(
    level: fearless_simd::Level,
    src: &[f32],
    from: IRect,
    to: IRect,
    taps: &[[f32; 3]],
    mode: u8,
) -> Vec<f32> {
    use rayon::prelude::*;
    let mut result = vec![0.0; to.width() * to.height()];
    let size = (from.width(), from.height());
    let (ox, oy) = ((to.x0 - from.x0) as f32, (to.y0 - from.y0) as f32);
    result
        .par_chunks_mut(to.width())
        .enumerate()
        .for_each(|(y, row)| {
            fearless_simd::dispatch!(level, _simd => {
                for (x, out) in row.iter_mut().enumerate() {
                    if mode == 2 {
                        *out = 1.0;
                    }
                    for &[dx, dy, weight] in taps {
                        let value = sample(src, size, ox + x as f32 + dx, oy + y as f32 + dy);
                        *out = match mode {
                            0 => value.mul_add(weight, *out),
                            1 => out.max(value),
                            _ => out.min(value),
                        };
                    }
                }
            });
        });
    result
}
