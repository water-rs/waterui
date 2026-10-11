//! Coverage convolution for silhouettes without an analytic shadow form.
use super::{FLATTEN_TOL, Item, Lowering, coverage_mask, flatten_edges};
use crate::names;
use crate::render::{glyph::GlyphMask, paint::PaintData};
use cherenkov::lowering::shadow::{capture_padding, gaussian_taps, spread_taps};
use cherenkov::lowering::shape_outline;
use cherenkov::{RenderError, Shadow, ShapeData};
use kurbo::Affine;
use std::sync::{Arc, OnceLock};

impl Lowering<'_, '_> {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
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
        let (px, py) = capture_padding(self.transform, sigma, shadow.spread);
        if !px.is_finite()
            || !py.is_finite()
            || px < 0.0
            || py < 0.0
            || px > f64::from(i32::MAX) / 4.0
            || py > f64::from(i32::MAX) / 4.0
        {
            return Err(RenderError::Render(
                "shadow capture exceeds addressable extent".into(),
            ));
        }
        let (px, py) = (px as usize, py as usize);
        let width = self
            .width
            .checked_add(2 * px)
            .ok_or_else(|| RenderError::Render("shadow width overflow".into()))?;
        let height = self
            .height
            .checked_add(2 * py)
            .ok_or_else(|| RenderError::Render("shadow height overflow".into()))?;
        width
            .checked_mul(height)
            .filter(|n| *n <= isize::MAX as usize / 4)
            .ok_or_else(|| RenderError::Render("shadow coverage overflow".into()))?;
        let scale = ma.hypot(mb).max(mc.hypot(md)).max(1e-12);
        let Some((path, rule)) = shape_outline(shape, FLATTEN_TOL / scale) else {
            return Err(RenderError::Unsupported(names::SHADOW));
        };
        let placement = Affine::translate((px as f64, py as f64))
            * self.transform
            * Affine::translate(shadow.offset);
        let edges = flatten_edges(placement * path, FLATTEN_TOL);
        let mut coverage = coverage_mask(&edges, rule, width, height);
        if shadow.spread != 0.0 {
            let taps = spread_taps([ma, mb, mc, md], shadow.spread, f64::from(i32::MAX))?;
            coverage = convolve(
                &coverage,
                width,
                height,
                &taps,
                if shadow.spread > 0.0 { 1 } else { 2 },
            );
        }
        if shadow.sigma > 0.0 {
            for axis in [[ma, mb], [mc, md]] {
                coverage = convolve(
                    &coverage,
                    width,
                    height,
                    &gaussian_taps(axis, sigma, f64::from(i32::MAX))?,
                    0,
                );
            }
        }
        // Retain only visible output. The padded contributors are temporary.
        let mut visible = Vec::with_capacity(self.width * self.height);
        for row in coverage.chunks_exact(width).skip(py).take(self.height) {
            visible.extend_from_slice(&row[px..px + self.width]);
        }
        let slot = Arc::new(OnceLock::new());
        slot.set(Arc::new(GlyphMask {
            left: 0,
            top: 0,
            w: self.width as u32,
            h: self.height as u32,
            cov: visible,
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

#[expect(clippy::cast_precision_loss, reason = "capture coordinates fit f32")]
fn convolve(src: &[f32], width: usize, height: usize, taps: &[[f32; 3]], mode: u8) -> Vec<f32> {
    use rayon::prelude::*;
    let mut result = vec![0.0; src.len()];
    result
        .par_chunks_mut(width)
        .enumerate()
        .for_each(|(y, row)| {
            for (x, out) in row.iter_mut().enumerate() {
                if mode == 2 {
                    *out = 1.0;
                }
                for &[dx, dy, weight] in taps {
                    let value = sample(src, (width, height), x as f32 + dx, y as f32 + dy);
                    *out = match mode {
                        0 => value.mul_add(weight, *out),
                        1 => out.max(value),
                        _ => out.min(value),
                    };
                }
            }
        });
    result
}
