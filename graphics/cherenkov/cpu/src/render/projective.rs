//! Projective layer images (#84): a projective layer's subtree is
//! rasterized into a layer-local premultiplied linear-P3 image, stored in
//! `f16` with a full area-average mip chain, and sampled through the
//! anisotropic filter when the layer composes into its parent.

use std::sync::Arc;

use cherenkov::lowering::projective::{Footprint, Homography, footprint_at};
use half::f16;

/// One mip level: `w × h` premultiplied `f16` texels.
pub struct Level {
    w: usize,
    h: usize,
    texels: Vec<[f16; 4]>,
}

/// A completed local image and its mip chain. Immutable once built, so
/// every band worker samples it concurrently.
pub struct ProjectedImage {
    levels: Vec<Level>,
}

impl ProjectedImage {
    /// Rounds the rasterized base level to `f16` and builds every mip
    /// level down to `1 × 1` by area averaging.
    #[must_use]
    pub fn build(base: &[[f32; 4]], size: (u32, u32)) -> Self {
        let (w, h) = (size.0 as usize, size.1 as usize);
        let mut levels = vec![Level {
            w,
            h,
            texels: base.iter().map(|px| px.map(f16::from_f32)).collect(),
        }];
        for &(nw, nh) in &cherenkov::lowering::projective::mip_levels(size)[1..] {
            let prev = levels.last().expect("the base level exists");
            levels.push(downsample(prev, nw as usize, nh as usize));
        }
        Self { levels }
    }

    /// Heap bytes: `8 · Σ w·h`.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.levels
            .iter()
            .map(|l| (l.texels.len() * size_of::<[f16; 4]>()) as u64)
            .sum()
    }

    /// The anisotropic reconstruction at the destination sample whose
    /// homogeneous preimage is `q = inverse · (d, 1)`: premultiplied
    /// colour, transparent without a front-facing preimage.
    #[must_use]
    pub fn sample(&self, inverse: &Homography, q: [f64; 3]) -> [f32; 4] {
        footprint_at(inverse, q).map_or([0.0; 4], |f| self.integrate(&f))
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "the level index is clamped to the chain"
    )]
    fn integrate(&self, footprint: &Footprint) -> [f32; 4] {
        let top = (self.levels.len() - 1) as f64;
        let lod = footprint.lod.clamp(0.0, top);
        let fine = lod.floor();
        let blend = lod - fine;
        let fine = fine as usize;
        let coarse = (fine + 1).min(self.levels.len() - 1);
        let base = &self.levels[0];
        let taps = f64::from(footprint.taps);
        let mut acc = [0.0_f64; 4];
        for tap in 0..footprint.taps {
            // Centres of `taps` equal parts of the major axis.
            let offset = 0.5f64.mul_add(-taps, f64::from(tap) + 0.5);
            let at = [
                offset.mul_add(footprint.step[0], footprint.center[0]),
                offset.mul_add(footprint.step[1], footprint.center[1]),
            ];
            let near = self.bilinear(fine, at, base);
            let texel = if blend > 0.0 && coarse != fine {
                let far = self.bilinear(coarse, at, base);
                std::array::from_fn(|c| blend.mul_add(far[c] - near[c], near[c]))
            } else {
                near
            };
            for (sum, value) in acc.iter_mut().zip(texel) {
                *sum += value;
            }
        }
        acc.map(|v| (v / taps) as f32)
    }

    /// Bilinear sample of `level` at base-texel point `s`; texels outside
    /// the level are transparent.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss,
        reason = "tap indices are range-checked against the level size"
    )]
    fn bilinear(&self, level: usize, at: [f64; 2], base: &Level) -> [f64; 4] {
        let lv = &self.levels[level];
        let u = at[0] * lv.w as f64 / base.w as f64 - 0.5;
        let v = at[1] * lv.h as f64 / base.h as f64 - 0.5;
        let (x0, y0) = (u.floor(), v.floor());
        let (fx, fy) = (u - x0, v - y0);
        let mut out = [0.0; 4];
        for (dy, wy) in [(0, 1.0 - fy), (1, fy)] {
            let y = y0 as i64 + dy;
            if wy == 0.0 || y < 0 || y >= lv.h as i64 {
                continue;
            }
            for (dx, wx) in [(0, 1.0 - fx), (1, fx)] {
                let x = x0 as i64 + dx;
                if wx == 0.0 || x < 0 || x >= lv.w as i64 {
                    continue;
                }
                let texel = lv.texels[y as usize * lv.w + x as usize];
                for c in 0..4 {
                    out[c] = (wx * wy).mul_add(f64::from(texel[c].to_f32()), out[c]);
                }
            }
        }
        out
    }
}

/// Area-overlap weights for resampling `from` texels onto `to` texels
/// spanning the same extent: destination texel `i` covers the source
/// interval `[i·s, (i+1)·s)`, `s = from / to`.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "level sizes are far below 2^24"
)]
fn weights(from: usize, to: usize) -> Vec<Vec<(usize, f32)>> {
    let s = from as f64 / to as f64;
    (0..to)
        .map(|i| {
            let (lo, hi) = (i as f64 * s, (i + 1) as f64 * s);
            (lo.floor() as usize..(hi.ceil() as usize).min(from))
                .filter_map(|j| {
                    let overlap = hi.min((j + 1) as f64) - lo.max(j as f64);
                    (overlap > 0.0).then(|| (j, (overlap / s) as f32))
                })
                .collect()
        })
        .collect()
}

/// The next mip level: a separable area average of the previous level's
/// decoded texels in premultiplied extended linear P3, never clamped,
/// rounded once to `f16`.
fn downsample(prev: &Level, w: usize, h: usize) -> Level {
    let (wx, wy) = (weights(prev.w, w), weights(prev.h, h));
    let mut rows = vec![[0.0_f32; 4]; w * prev.h];
    for y in 0..prev.h {
        for (x, taps) in wx.iter().enumerate() {
            let mut acc = [0.0_f32; 4];
            for &(j, weight) in taps {
                let texel = prev.texels[y * prev.w + j];
                for c in 0..4 {
                    acc[c] = weight.mul_add(texel[c].to_f32(), acc[c]);
                }
            }
            rows[y * w + x] = acc;
        }
    }
    let mut texels = vec![[f16::ZERO; 4]; w * h];
    for (y, taps) in wy.iter().enumerate() {
        for x in 0..w {
            let mut acc = [0.0_f32; 4];
            for &(j, weight) in taps {
                let px = rows[j * w + x];
                for c in 0..4 {
                    acc[c] = weight.mul_add(px[c], acc[c]);
                }
            }
            texels[y * w + x] = acc.map(f16::from_f32);
        }
    }
    Level { w, h, texels }
}

/// What a retained local image was realized from. The layer's pose,
/// opacity and blend are absent: changing them only moves the sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    /// Texels per layer unit.
    pub density: f64,
    /// Layer space to base texels (the stable texel-grid origin).
    local_to_texel: [f64; 6],
    size: (u32, u32),
    /// [`cherenkov::SurfaceTree::content_stamp`] of the layer.
    stamp: u64,
    /// The renderer's image-replacement count.
    replacements: u64,
}

impl Key {
    /// The key of `layout` at content stamp `stamp` after `replacements`
    /// image replacements.
    #[must_use]
    pub const fn new(
        layout: &cherenkov::lowering::projective::LocalImage,
        stamp: u64,
        replacements: u64,
    ) -> Self {
        Self {
            density: layout.density,
            local_to_texel: layout.local_to_texel.as_coeffs(),
            size: layout.size,
            stamp,
            replacements,
        }
    }

    /// Whether the image was realized from the layer's content at
    /// `stamp` after `replacements` image replacements. Stamps and the
    /// count only grow, so an image that is not current can never be
    /// composed again.
    #[must_use]
    pub const fn is_current(&self, stamp: u64, replacements: u64) -> bool {
        self.stamp == stamp && self.replacements == replacements
    }
}

/// One retained realization.
pub struct Entry {
    /// What it was realized from.
    pub key: Key,
    /// The completed image.
    pub image: Arc<ProjectedImage>,
    /// Filters its rendering ran; an animating one forces a new
    /// realization.
    pub filters: Vec<u64>,
    /// Backdrop groups its rendering captured.
    pub groups: Vec<u64>,
    /// The renderer frame count that last composed it.
    pub last_used: u64,
}

/// The filters and backdrop groups a frame's rasters used.
#[derive(Default)]
pub struct Used {
    /// Filter ids.
    pub filters: rustc_hash::FxHashSet<u64>,
    /// Backdrop group ids.
    pub groups: rustc_hash::FxHashSet<u64>,
}

impl Used {
    /// Adds one lowering's sets, moving them in when this set is empty.
    pub fn absorb(
        &mut self,
        filters: rustc_hash::FxHashSet<u64>,
        groups: rustc_hash::FxHashSet<u64>,
    ) {
        if self.filters.is_empty() {
            self.filters = filters;
        } else {
            self.filters.extend(filters);
        }
        if self.groups.is_empty() {
            self.groups = groups;
        } else {
            self.groups.extend(groups);
        }
    }
}

/// A projected image placed in its parent raster for one frame.
#[derive(Clone)]
pub struct Placed {
    /// The completed local image.
    pub image: Arc<ProjectedImage>,
    /// Parent pixels to base texels, front half-space at `w > 0`.
    pub inverse: Homography,
    /// Parent-raster pixels to shade.
    pub bounds: [u32; 4],
    /// Layer space to parent raster pixels.
    pub to_parent: Homography,
    /// The image's texels per layer unit.
    pub density: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use cherenkov::kurbo::Affine;

    #[test]
    fn odd_levels_average_by_area_overlap_without_dropping_edges() {
        let base: Vec<[f32; 4]> = [0.0, 1.0, 2.0, 3.0, 4.0]
            .map(|v| [v, 0.0, 0.0, 1.0])
            .to_vec();
        let image = ProjectedImage::build(&base, (5, 1));
        // 5 → 2: [0, 2.5) and [2.5, 5); the middle texel splits evenly.
        let l1: Vec<f32> = image.levels[1]
            .texels
            .iter()
            .map(|t| t[0].to_f32())
            .collect();
        assert_eq!(
            l1,
            [(0.0 + 1.0 + 1.0) / 2.5, (1.0 + 3.0 + 4.0) / 2.5]
                .map(|v: f32| f16::from_f32(v).to_f32())
        );
        let l2 = image.levels[2].texels[0][0].to_f32();
        assert!((l2 - 2.0).abs() < 1e-3, "{l2}");
    }

    #[test]
    fn outside_the_source_domain_is_transparent_at_every_level() {
        let base = vec![[1.0, 1.0, 1.0, 1.0]; 16 * 16];
        let image = ProjectedImage::build(&base, (16, 16));
        for level in 0..image.levels.len() {
            // The domain edge is half covered at any level.
            let edge = image.bilinear(level, [0.0, 8.0], &image.levels[0])[3];
            assert!((edge - 0.5).abs() < 1e-6, "level {level}: {edge}");
            let outside = image.bilinear(level, [-40.0, 8.0], &image.levels[0])[3];
            assert_eq!(outside.to_bits(), 0.0_f64.to_bits());
        }
    }

    #[test]
    fn an_identity_projection_reproduces_texels() {
        let base: Vec<[f32; 4]> = (0..64_u16)
            .map(|i| [f32::from(i % 8) / 8.0, 0.25, 0.5, 1.0])
            .collect();
        let image = ProjectedImage::build(&base, (8, 8));
        let inverse = Homography::affine(Affine::IDENTITY).front_inverse();
        for (i, want) in (0..64_u16).zip(&base) {
            let d = [f64::from(i % 8) + 0.5, f64::from(i / 8) + 0.5];
            let got = image.sample(&inverse, inverse.map(d[0], d[1]));
            for c in 0..4 {
                assert!((got[c] - f16::from_f32(want[c]).to_f32()).abs() < 1e-6);
            }
        }
    }
}
