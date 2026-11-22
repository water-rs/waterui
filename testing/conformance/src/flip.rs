//! Pure-Rust cross-platform NVIDIA LDR-FLIP perceptual diff engine and magma
//! heatmap generator over RGBA8 images.
//!
//! Algorithm ported from `FLIP.h` in <https://github.com/NVlabs/flip>
//! (BSD-3-Clause):
//!
//! - Andersson, Nilsson, Akenine-Möller, Oskarsson, Åström, Fairchild.
//!   "FLIP: A Difference Evaluator for Alternating Images", HPG 2020.
//!
//! Operates directly on pure-Rust RGBA8 buffers with no platform-specific image
//! decoders. Pixels are decoded with the IEC 61966-2-1 sRGB EOTF and
//! premultiplied by alpha (equivalent to compositing over black) before FLIP
//! spatial filtering, while [`FlipMetrics::max_local_error`] filters all four
//! premultiplied RGBA channels with a 3×3 box filter so alpha-only differences
//! are also detected.

use core::f64::consts::{FRAC_1_SQRT_2, PI};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::error::ConformanceError;
use crate::image::RgbaImage;

/// Default pixels per degree from NVIDIA FLIP (`0.7 m` viewing distance,
/// `3840 px` display across `0.7 m` width → `≈ 67.02`).
pub const DEFAULT_PPD: f64 = 0.7 * (3840.0 / 0.7) * (PI / 180.0);

/// Default sliding-block edge length (in pixels) used for
/// [`FlipMetrics::max_block_flip_mean`].
pub const DEFAULT_BLOCK_SIZE: usize = 16;

// Verbatim reference constants from NVlabs/flip `FLIP.h`.
const GQC: f64 = 0.7;
const GPC: f64 = 0.4;
const GPT: f64 = 0.95;
const GW: f64 = 0.082;

const A1: [f64; 3] = [1.0, 1.0, 34.1];
const B1: [f64; 3] = [0.0047, 0.0053, 0.04];
const A2: [f64; 3] = [0.0, 0.0, 13.5];
const B2: [f64; 3] = [1.0e-5, 1.0e-5, 0.025];

#[expect(
    clippy::unreadable_literal,
    reason = "verbatim D65 illuminant constants from NVlabs/flip FLIP.h"
)]
const INV_ILLUMINANT: [f64; 3] = [1.052156925, 1.0, 0.918357670];

#[expect(
    clippy::unreadable_literal,
    reason = "verbatim D65 illuminant constants from NVlabs/flip FLIP.h"
)]
const ILLUMINANT: [f64; 3] = [0.950428545, 1.0, 1.088900371];

/// Linear sRGB (BT.709 primaries, D65) to CIE XYZ.
const SRGB_TO_XYZ: [[f64; 3]; 3] = [
    [
        0.412_390_799_265_959_4,
        0.357_584_339_383_878,
        0.180_480_788_401_834_3,
    ],
    [
        0.212_639_005_871_510_4,
        0.715_168_678_767_756,
        0.072_192_315_360_733_7,
    ],
    [
        0.019_330_818_715_591_8,
        0.119_194_779_794_626,
        0.950_532_152_249_660_7,
    ],
];

/// Linear Display P3 (D65) to CIE XYZ.
const P3_TO_XYZ: [[f64; 3]; 3] = [
    [
        0.486_570_948_648_216_2,
        0.265_667_693_169_093_1,
        0.198_217_285_234_362_5,
    ],
    [
        0.228_974_564_069_748_8,
        0.691_738_521_836_506_2,
        0.079_286_914_093_745_0,
    ],
    [0.0, 0.045_113_381_858_902_6, 1.043_944_368_900_976],
];

/// 256-entry Magma colormap LUT verbatim from `MapMagma` in NVlabs/flip `FLIP.h`.
#[expect(
    clippy::unreadable_literal,
    reason = "verbatim Magma LUT from NVlabs/flip FLIP.h"
)]
const MAGMA: [[f64; 3]; 256] = include!("magma_table.rs");

/// Colour gamut of the RGBA8 inputs supplied to FLIP.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ColorGamut {
    /// Standard sRGB (BT.709 primaries, sRGB transfer function).
    #[default]
    Srgb,
    /// Display P3 (DCI-P3 D65 primaries, sRGB transfer function).
    DisplayP3,
}

impl ColorGamut {
    const fn to_xyz_matrix(self) -> [[f64; 3]; 3] {
        match self {
            Self::Srgb => SRGB_TO_XYZ,
            Self::DisplayP3 => P3_TO_XYZ,
        }
    }
}

/// Configuration for NVIDIA LDR-FLIP perceptual image comparison.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FlipConfig {
    /// Pixels per degree of visual angle (defaults to [`DEFAULT_PPD`]).
    pub ppd: f64,
    /// Sliding-window tile edge length (in pixels) for
    /// [`FlipMetrics::max_block_flip_mean`] (defaults to [`DEFAULT_BLOCK_SIZE`]).
    pub block_size: usize,
    /// Gamut of the input RGBA8 PNG buffers (defaults to [`ColorGamut::Srgb`]).
    pub gamut: ColorGamut,
}

impl Default for FlipConfig {
    fn default() -> Self {
        Self {
            ppd: DEFAULT_PPD,
            block_size: DEFAULT_BLOCK_SIZE,
            gamut: ColorGamut::Srgb,
        }
    }
}

/// Pass/fail ceilings for [`FlipMetrics`].
///
/// Any `None` bound is unchecked.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FlipThresholds {
    /// Ceiling on [`FlipMetrics::flip_mean`].
    pub max_flip_mean: Option<f64>,
    /// Ceiling on [`FlipMetrics::flip_p95`].
    pub max_flip_p95: Option<f64>,
    /// Ceiling on [`FlipMetrics::flip_max`].
    pub max_flip_max: Option<f64>,
    /// Ceiling on [`FlipMetrics::max_block_flip_mean`].
    pub max_block_flip_mean: Option<f64>,
    /// Ceiling on [`FlipMetrics::max_local_error`].
    pub max_local_error: Option<f64>,
}

impl FlipThresholds {
    /// Strict equivalence thresholds suitable for comparing two routes to the
    /// same visual specification (for example GPU vs CPU painters).
    #[must_use]
    pub const fn strict() -> Self {
        Self {
            max_flip_mean: Some(0.006),
            max_flip_p95: Some(0.03),
            max_flip_max: Some(0.15),
            max_block_flip_mean: Some(0.05),
            max_local_error: Some(0.09),
        }
    }

    /// Relaxed thresholds suitable for cross-backend visual parity (for
    /// example native AppKit/UIKit vs self-drawn Hydrolysis) where font
    /// rasterization and subpixel anti-aliasing differ slightly.
    #[must_use]
    pub const fn cross_backend_parity() -> Self {
        Self {
            max_flip_mean: Some(0.04),
            max_flip_p95: Some(0.20),
            max_flip_max: None,
            max_block_flip_mean: Some(0.18),
            max_local_error: None,
        }
    }

    /// Evaluates `metrics` against these thresholds, returning human-readable
    /// failure descriptions (empty when every configured threshold passes).
    #[must_use]
    pub fn evaluate(&self, metrics: &FlipMetrics) -> Vec<String> {
        let mut failures = Vec::new();
        if let Some(limit) = self.max_flip_mean
            && metrics.flip_mean > limit
        {
            failures.push(format!(
                "flip_mean {:.4} exceeds limit {limit:.4}",
                metrics.flip_mean
            ));
        }
        if let Some(limit) = self.max_flip_p95
            && metrics.flip_p95 > limit
        {
            failures.push(format!(
                "flip_p95 {:.4} exceeds limit {limit:.4}",
                metrics.flip_p95
            ));
        }
        if let Some(limit) = self.max_flip_max
            && metrics.flip_max > limit
        {
            failures.push(format!(
                "flip_max {:.4} exceeds limit {limit:.4}",
                metrics.flip_max
            ));
        }
        if let Some(limit) = self.max_block_flip_mean
            && metrics.max_block_flip_mean > limit
        {
            failures.push(format!(
                "max_block_flip_mean {:.4} exceeds limit {limit:.4}",
                metrics.max_block_flip_mean
            ));
        }
        if let Some(limit) = self.max_local_error
            && metrics.max_local_error > limit
        {
            failures.push(format!(
                "max_local_error {:.4} exceeds limit {limit:.4}",
                metrics.max_local_error
            ));
        }
        failures
    }
}

impl Default for FlipThresholds {
    fn default() -> Self {
        Self::strict()
    }
}

/// Summary perceptual error metrics returned by [`compare_flip`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FlipMetrics {
    /// Arithmetic mean of per-pixel FLIP error across the image (`0.0..=1.0`).
    pub flip_mean: f64,
    /// 95th percentile per-pixel FLIP error across the image (`0.0..=1.0`).
    pub flip_p95: f64,
    /// Maximum per-pixel FLIP error across the image (`0.0..=1.0`).
    pub flip_max: f64,
    /// Worst mean FLIP error over any sliding `block_size × block_size` window
    /// (catches localized component regressions without being noisy on 1px AA).
    pub max_block_flip_mean: f64,
    /// Maximum per-channel absolute difference between 3×3 box-filtered
    /// premultiplied linear RGBA channels.
    pub max_local_error: f64,
}

/// Full FLIP comparison result including summary [`FlipMetrics`] and the
/// per-pixel error map for heatmap rendering.
#[derive(Debug, Clone)]
pub struct FlipResult {
    width: u32,
    height: u32,
    metrics: FlipMetrics,
    error_map: Vec<f64>,
}

impl FlipResult {
    /// Image width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Image height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Summary perceptual metrics.
    #[must_use]
    pub const fn metrics(&self) -> FlipMetrics {
        self.metrics
    }

    /// Row-major per-pixel FLIP error map (`width * height` entries in `0.0..=1.0`).
    #[must_use]
    pub const fn error_map(&self) -> &[f64] {
        self.error_map.as_slice()
    }

    /// Encodes the per-pixel error map into an opaque RGBA8 Magma heatmap image.
    #[must_use]
    pub fn magma_heatmap(&self) -> RgbaImage {
        magma_heatmap(&self.error_map, self.width, self.height)
    }
}

/// Compares two RGBA8 images with NVIDIA LDR-FLIP using [`FlipConfig::default`].
///
/// # Errors
///
/// Returns [`ConformanceError::DimensionMismatch`] when the images differ in
/// size or [`ConformanceError::EmptyImage`] when either dimension is zero.
pub fn compare_flip(
    reference: &RgbaImage,
    candidate: &RgbaImage,
) -> Result<FlipResult, ConformanceError> {
    compare_flip_with(reference, candidate, FlipConfig::default())
}

/// Compares two RGBA8 images with NVIDIA LDR-FLIP using an explicit
/// [`FlipConfig`].
///
/// # Errors
///
/// Returns [`ConformanceError::InvalidConfig`] for invalid parameters,
/// [`ConformanceError::DimensionMismatch`] when the images differ in size, or
/// [`ConformanceError::EmptyImage`] when either dimension is zero.
#[expect(
    clippy::cast_precision_loss,
    reason = "pixel counts are far below 2^53"
)]
pub fn compare_flip_with(
    reference: &RgbaImage,
    candidate: &RgbaImage,
    config: FlipConfig,
) -> Result<FlipResult, ConformanceError> {
    if !config.ppd.is_finite() || config.ppd <= 0.0 {
        return Err(ConformanceError::InvalidConfig(
            "ppd must be finite and greater than zero",
        ));
    }
    if config.block_size == 0 {
        return Err(ConformanceError::InvalidConfig(
            "block_size must be at least 1",
        ));
    }
    if reference.width() != candidate.width() || reference.height() != candidate.height() {
        return Err(ConformanceError::DimensionMismatch {
            ref_width: reference.width(),
            ref_height: reference.height(),
            test_width: candidate.width(),
            test_height: candidate.height(),
        });
    }
    if reference.width() == 0 || reference.height() == 0 {
        return Err(ConformanceError::EmptyImage {
            width: reference.width(),
            height: reference.height(),
        });
    }

    let width = reference.width() as usize;
    let height = reference.height() as usize;
    let ref_linear = decode_premultiplied_linear(reference.rgba8());
    let cand_linear = decode_premultiplied_linear(candidate.rgba8());

    let filters = Filters::new(config.ppd, config.gamut);
    let ref_features = Features::new(&ref_linear, width, height, &filters);
    let cand_features = Features::new(&cand_linear, width, height, &filters);
    let error_map = flip_error(&ref_features, &cand_features, &filters);

    let mut sum = 0.0;
    let mut max = 0.0_f64;
    for &e in &error_map {
        sum += e;
        max = max.max(e);
    }
    let flip_mean = sum / error_map.len() as f64;
    let flip_p95 = percentile(&error_map, 0.95);
    let max_block_flip_mean = sliding_block_max_mean(&error_map, width, height, config.block_size);
    let max_local_error = local_error(
        &box3x3(&ref_linear, width, height),
        &box3x3(&cand_linear, width, height),
    );

    Ok(FlipResult {
        width: reference.width(),
        height: reference.height(),
        metrics: FlipMetrics {
            flip_mean,
            flip_p95,
            flip_max: max,
            max_block_flip_mean,
            max_local_error,
        },
        error_map,
    })
}

/// Maps a row-major slice of per-pixel FLIP errors (`0.0..=1.0`) to an opaque
/// RGBA8 Magma heatmap image.
///
/// # Panics
///
/// Panics if `error_map.len() != width * height`.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "error and magma channels are clamped to [0, 1] before u8 quantization"
)]
pub fn magma_heatmap(error_map: &[f64], width: u32, height: u32) -> RgbaImage {
    let expected = (width as usize) * (height as usize);
    assert_eq!(
        error_map.len(),
        expected,
        "error_map length ({}) does not match {}x{}",
        error_map.len(),
        width,
        height
    );
    let mut rgba8 = Vec::with_capacity(expected * 4);
    for &e in error_map {
        let idx = (e.clamp(0.0, 1.0) * 255.0).round() as usize;
        let color = MAGMA[idx.min(255)];
        rgba8.push((color[0] * 255.0).round() as u8);
        rgba8.push((color[1] * 255.0).round() as u8);
        rgba8.push((color[2] * 255.0).round() as u8);
        rgba8.push(255);
    }
    RgbaImage::new(width, height, rgba8)
        .expect("magma_heatmap allocates exactly width * height * 4 bytes")
}

/// IEC 61966-2-1 sRGB EOTF (encoded `[0, 1]` -> linear `[0, 1]`).
fn srgb_to_linear(encoded: f64) -> f64 {
    if encoded <= 0.04045 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    }
}

/// Decodes RGBA8 straight sRGB pixels into premultiplied linear `[r, g, b, a]`.
fn decode_premultiplied_linear(rgba8: &[u8]) -> Vec<[f64; 4]> {
    let mut lut = [0.0_f64; 256];
    for (i, slot) in lut.iter_mut().enumerate() {
        *slot = srgb_to_linear(f64::from(u8::try_from(i).expect("index fits in u8")) / 255.0);
    }
    rgba8
        .par_chunks_exact(4)
        .map(|px| {
            let a = f64::from(px[3]) / 255.0;
            [
                lut[usize::from(px[0])] * a,
                lut[usize::from(px[1])] * a,
                lut[usize::from(px[2])] * a,
                a,
            ]
        })
        .collect()
}

fn mat3_mul(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [
        m[0][2].mul_add(v[2], m[0][1].mul_add(v[1], m[0][0] * v[0])),
        m[1][2].mul_add(v[2], m[1][1].mul_add(v[1], m[1][0] * v[0])),
        m[2][2].mul_add(v[2], m[2][1].mul_add(v[1], m[2][0] * v[0])),
    ]
}

fn mat3_inv(m: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let det = m[0][2].mul_add(
        m[1][1].mul_add(-m[2][0], m[1][0] * m[2][1]),
        m[0][1].mul_add(
            -m[1][2].mul_add(-m[2][0], m[1][0] * m[2][2]),
            m[0][0] * m[1][2].mul_add(-m[2][1], m[1][1] * m[2][2]),
        ),
    );
    let inv_det = 1.0 / det;
    let mut inv = [[0.0; 3]; 3];
    inv[0][0] = m[1][2].mul_add(-m[2][1], m[1][1] * m[2][2]) * inv_det;
    inv[0][1] = m[0][1].mul_add(-m[2][2], m[0][2] * m[2][1]) * inv_det;
    inv[0][2] = m[0][2].mul_add(-m[1][1], m[0][1] * m[1][2]) * inv_det;
    inv[1][0] = m[1][0].mul_add(-m[2][2], m[1][2] * m[2][0]) * inv_det;
    inv[1][1] = m[0][2].mul_add(-m[2][0], m[0][0] * m[2][2]) * inv_det;
    inv[1][2] = m[0][0].mul_add(-m[1][2], m[0][2] * m[1][0]) * inv_det;
    inv[2][0] = m[1][1].mul_add(-m[2][0], m[1][0] * m[2][1]) * inv_det;
    inv[2][1] = m[0][0].mul_add(-m[2][1], m[0][1] * m[2][0]) * inv_det;
    inv[2][2] = m[0][1].mul_add(-m[1][0], m[0][0] * m[1][1]) * inv_det;
    inv
}

fn xyz_to_ycxcz(xyz: [f64; 3]) -> [f64; 3] {
    let x = xyz[0] * INV_ILLUMINANT[0];
    let y = xyz[1] * INV_ILLUMINANT[1];
    let z = xyz[2] * INV_ILLUMINANT[2];
    [
        116.0_f64.mul_add(y, -16.0),
        500.0 * (x - y),
        200.0 * (y - z),
    ]
}

fn ycxcz_to_xyz(ycxcz: [f64; 3]) -> [f64; 3] {
    let y = (ycxcz[0] + 16.0) / 116.0;
    let cx = ycxcz[1] / 500.0;
    let cz = ycxcz[2] / 200.0;
    [
        (y + cx) * ILLUMINANT[0],
        y * ILLUMINANT[1],
        (y - cz) * ILLUMINANT[2],
    ]
}

fn xyz_to_cielab(xyz: [f64; 3]) -> [f64; 3] {
    let delta = 6.0 / 29.0;
    let delta_cube = delta * delta * delta;
    let factor = 1.0 / (3.0 * delta * delta);
    let term = 4.0 / 29.0;
    let mut v = [
        xyz[0] * INV_ILLUMINANT[0],
        xyz[1] * INV_ILLUMINANT[1],
        xyz[2] * INV_ILLUMINANT[2],
    ];
    for c in &mut v {
        *c = if *c > delta_cube {
            c.powf(1.0 / 3.0)
        } else {
            factor.mul_add(*c, term)
        };
    }
    [
        116.0_f64.mul_add(v[1], -16.0),
        500.0 * (v[0] - v[1]),
        200.0 * (v[1] - v[2]),
    ]
}

fn hunt(luminance: f64, chrominance: f64) -> f64 {
    0.01 * luminance * chrominance
}

fn hyab(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] - b[0]).abs() + (a[1] - b[1]).hypot(a[2] - b[2])
}

fn gaussian(x2: f64, a: f64, b: f64) -> f64 {
    a * (PI / b).sqrt() * (-PI * PI * x2 / b).exp()
}

fn gaussian_sqrt(x2: f64, a: f64, b: f64) -> f64 {
    (a * (PI / b).sqrt()).sqrt() * (-PI * PI * x2 / b).exp()
}

fn compute_max_distance(rgb_to_xyz: &[[f64; 3]; 3]) -> f64 {
    let lab = |rgb: [f64; 3]| xyz_to_cielab(mat3_mul(rgb_to_xyz, rgb));
    let g = lab([0.0, 1.0, 0.0]);
    let b = lab([0.0, 0.0, 1.0]);
    let gh = [g[0], hunt(g[0], g[1]), hunt(g[0], g[2])];
    let bh = [b[0], hunt(b[0], b[1]), hunt(b[0], b[2])];
    hyab(gh, bh).powf(GQC)
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "filter radius and tap indices are small positive values"
)]
fn spatial_filters(ppd: f64) -> (Vec<[f64; 2]>, Vec<[f64; 2]>) {
    let max_b = B1[0].max(B1[1]).max(B1[2]).max(B2[0]).max(B2[1]).max(B2[2]);
    let radius = (3.0 * (max_b / (2.0 * PI * PI)).sqrt() * ppd).ceil() as usize;
    let dx = 1.0 / ppd;
    let width = 2 * radius + 1;
    let mut fycx = Vec::with_capacity(width);
    let mut fcz = Vec::with_capacity(width);
    let (mut sum_y, mut sum_cx, mut sum_cz1, mut sum_cz2) = (0.0, 0.0, 0.0, 0.0);
    for i in 0..width {
        let ix = (i as f64 - radius as f64) * dx;
        let ix2 = ix * ix;
        let y = gaussian(ix2, A1[0], B1[0]);
        let cx = gaussian(ix2, A1[1], B1[1]);
        let cz1 = gaussian_sqrt(ix2, A1[2], B1[2]);
        let cz2 = gaussian_sqrt(ix2, A2[2], B2[2]);
        fycx.push([y, cx]);
        fcz.push([cz1, cz2]);
        sum_y += y;
        sum_cx += cx;
        sum_cz1 += cz1;
        sum_cz2 += cz2;
    }
    for w in &mut fycx {
        w[0] /= sum_y;
        w[1] /= sum_cx;
    }
    let norm = 1.0 / sum_cz1.hypot(sum_cz2);
    for w in &mut fcz {
        w[0] *= norm;
        w[1] *= norm;
    }
    (fycx, fcz)
}

#[expect(
    clippy::similar_names,
    reason = "variable names mirror the NVlabs/flip reference algorithm"
)]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "filter radius and tap indices are small positive values"
)]
fn feature_filter(ppd: f64) -> Vec<[f64; 3]> {
    let stddev = 0.5 * GW * ppd;
    let radius = (3.0 * stddev).ceil() as usize;
    let width = 2 * radius + 1;
    let mut filter = Vec::with_capacity(width);
    let (mut g_sum, mut dg_pos, mut dg_neg, mut ddg_pos, mut ddg_neg) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for i in 0..width {
        let x = i as f64 - radius as f64;
        let g = (-x * x / (2.0 * stddev * stddev)).exp();
        g_sum += g;
        let dg = -x * g;
        if dg > 0.0 {
            dg_pos += dg;
        } else {
            dg_neg -= dg;
        }
        let ddg = (x * x / (stddev * stddev) - 1.0) * g;
        if ddg > 0.0 {
            ddg_pos += ddg;
        } else {
            ddg_neg -= ddg;
        }
        filter.push([g, dg, ddg]);
    }
    for w in &mut filter {
        w[0] /= g_sum;
        w[1] /= if w[1] > 0.0 { dg_pos } else { dg_neg };
        w[2] /= if w[2] > 0.0 { ddg_pos } else { ddg_neg };
    }
    filter
}

struct Filters {
    ycx: Vec<[f64; 2]>,
    cz: Vec<[f64; 2]>,
    feature: Vec<[f64; 3]>,
    rgb_to_xyz: [[f64; 3]; 3],
    xyz_to_rgb: [[f64; 3]; 3],
    cmax: f64,
    pccmax: f64,
}

impl Filters {
    fn new(ppd: f64, gamut: ColorGamut) -> Self {
        let (ycx, cz) = spatial_filters(ppd);
        let rgb_to_xyz = gamut.to_xyz_matrix();
        let cmax = compute_max_distance(&rgb_to_xyz);
        Self {
            ycx,
            cz,
            feature: feature_filter(ppd),
            rgb_to_xyz,
            xyz_to_rgb: mat3_inv(&rgb_to_xyz),
            cmax,
            pccmax: GPC * cmax,
        }
    }
}

struct Features {
    hunt: Vec<[f64; 3]>,
    edge_point: Vec<[f64; 2]>,
}

impl Features {
    fn new(rgba: &[[f64; 4]], width: usize, height: usize, filters: &Filters) -> Self {
        let ycxcz: Vec<[f64; 3]> = rgba
            .par_iter()
            .map(|px| xyz_to_ycxcz(mat3_mul(&filters.rgb_to_xyz, [px[0], px[1], px[2]])))
            .collect();
        Self {
            hunt: filtered_hunt(&ycxcz, width, height, filters),
            edge_point: edge_point(&ycxcz, width, height, &filters.feature),
        }
    }
}

fn filtered_hunt(
    ycxcz: &[[f64; 3]],
    width: usize,
    height: usize,
    filters: &Filters,
) -> Vec<[f64; 3]> {
    let radius = filters.ycx.len() / 2;
    let taps = || filters.ycx.iter().zip(&filters.cz).enumerate();

    let mut x_pass = vec![([0.0; 2], [0.0; 2]); ycxcz.len()];
    x_pass
        .par_chunks_mut(width)
        .zip(ycxcz.par_chunks(width))
        .for_each(|(row, src)| {
            for (x, out) in row.iter_mut().enumerate() {
                let (mut sycx, mut scz) = ([0.0; 2], [0.0; 2]);
                for (k, (&[wy, wc], &[wz1, wz2])) in taps() {
                    let c = src[(x + k).saturating_sub(radius).min(width - 1)];
                    sycx[0] = wy.mul_add(c[0], sycx[0]);
                    sycx[1] = wc.mul_add(c[1], sycx[1]);
                    scz[0] = wz1.mul_add(c[2], scz[0]);
                    scz[1] = wz2.mul_add(c[2], scz[1]);
                }
                *out = (sycx, scz);
            }
        });

    let mut hunted = vec![[0.0; 3]; ycxcz.len()];
    hunted
        .par_chunks_mut(width)
        .enumerate()
        .for_each(|(y, row)| {
            for (x, out) in row.iter_mut().enumerate() {
                let (mut sycx, mut scz) = ([0.0; 2], [0.0; 2]);
                for (k, (&[wy, wc], &[wz1, wz2])) in taps() {
                    let yy = (y + k).saturating_sub(radius).min(height - 1);
                    let (ycx, cz) = x_pass[yy * width + x];
                    sycx[0] = wy.mul_add(ycx[0], sycx[0]);
                    sycx[1] = wc.mul_add(ycx[1], sycx[1]);
                    scz[0] = wz1.mul_add(cz[0], scz[0]);
                    scz[1] = wz2.mul_add(cz[1], scz[1]);
                }
                let rgb = mat3_mul(
                    &filters.xyz_to_rgb,
                    ycxcz_to_xyz([sycx[0], sycx[1], scz[0] + scz[1]]),
                )
                .map(|v| v.clamp(0.0, 1.0));
                let lab = xyz_to_cielab(mat3_mul(&filters.rgb_to_xyz, rgb));
                *out = [lab[0], hunt(lab[0], lab[1]), hunt(lab[0], lab[2])];
            }
        });
    hunted
}

fn edge_point(
    ycxcz: &[[f64; 3]],
    width: usize,
    height: usize,
    filter: &[[f64; 3]],
) -> Vec<[f64; 2]> {
    let radius = filter.len() / 2;
    let mut x_pass = vec![[0.0; 3]; ycxcz.len()];
    x_pass
        .par_chunks_mut(width)
        .zip(ycxcz.par_chunks(width))
        .for_each(|(row, src)| {
            for (x, out) in row.iter_mut().enumerate() {
                let (mut dx, mut ddx, mut g) = (0.0, 0.0, 0.0);
                for (k, w) in filter.iter().enumerate() {
                    let xx = (x + k).saturating_sub(radius).min(width - 1);
                    let yn = src[xx][0] / 116.0 + 16.0 / 116.0;
                    dx = w[1].mul_add(yn, dx);
                    ddx = w[2].mul_add(yn, ddx);
                    g = w[0].mul_add(yn, g);
                }
                *out = [dx, ddx, g];
            }
        });

    let mut magnitudes = vec![[0.0; 2]; ycxcz.len()];
    magnitudes
        .par_chunks_mut(width)
        .enumerate()
        .for_each(|(y, row)| {
            for (x, out) in row.iter_mut().enumerate() {
                let mut d = [0.0; 4];
                for (k, w) in filter.iter().enumerate() {
                    let yy = (y + k).saturating_sub(radius).min(height - 1);
                    let p = x_pass[yy * width + x];
                    d[0] = w[0].mul_add(p[0], d[0]);
                    d[1] = w[0].mul_add(p[1], d[1]);
                    d[2] = w[1].mul_add(p[2], d[2]);
                    d[3] = w[2].mul_add(p[2], d[3]);
                }
                *out = [d[0].hypot(d[2]), d[1].hypot(d[3])];
            }
        });
    magnitudes
}

fn flip_error(reference: &Features, candidate: &Features, filters: &Filters) -> Vec<f64> {
    reference
        .hunt
        .par_iter()
        .zip(&candidate.hunt)
        .zip(reference.edge_point.par_iter().zip(&candidate.edge_point))
        .map(|((hr, ht), (fr, ft))| pixel_error(*hr, *ht, *fr, *ft, filters))
        .collect()
}

fn pixel_error(
    reference_hunt: [f64; 3],
    candidate_hunt: [f64; 3],
    reference_features: [f64; 2],
    candidate_features: [f64; 2],
    filters: &Filters,
) -> f64 {
    let (cmax, pccmax) = (filters.cmax, filters.pccmax);
    let mut cd = hyab(reference_hunt, candidate_hunt).powf(GQC);
    cd = if cd < pccmax {
        cd * (GPT / pccmax)
    } else {
        ((cd - pccmax) / (cmax - pccmax)).mul_add(1.0 - GPT, GPT)
    };
    let edge_diff = (reference_features[0] - candidate_features[0]).abs();
    let point_diff = (reference_features[1] - candidate_features[1]).abs();
    let fd = (edge_diff.max(point_diff) * FRAC_1_SQRT_2).sqrt();
    cd.powf(1.0 - fd)
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "percentile rank index is bounded within [0, values.len() - 1]"
)]
fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable_by(f64::total_cmp);
    let pos = (sorted.len() - 1) as f64 * p.clamp(0.0, 1.0);
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    let frac = pos - lo as f64;
    sorted[lo].mul_add(1.0 - frac, sorted[hi] * frac)
}

/// Computes the maximum mean FLIP error over any sliding `bw × bh` window using
/// a summed-area table (`O(width * height)`).
#[expect(
    clippy::cast_precision_loss,
    reason = "block pixel area is far below 2^53"
)]
fn sliding_block_max_mean(
    error_map: &[f64],
    width: usize,
    height: usize,
    block_size: usize,
) -> f64 {
    let bw = block_size.min(width).max(1);
    let bh = block_size.min(height).max(1);
    let stride = width + 1;
    let mut sat = vec![0.0_f64; stride * (height + 1)];

    for y in 0..height {
        let mut row_sum = 0.0;
        let src_row = y * width;
        let cur_row = (y + 1) * stride;
        let prev_row = y * stride;
        for x in 0..width {
            row_sum += error_map[src_row + x];
            sat[cur_row + x + 1] = sat[prev_row + x + 1] + row_sum;
        }
    }

    let area = (bw * bh) as f64;
    let mut best = 0.0_f64;
    for y in 0..=(height - bh) {
        let y0 = y * stride;
        let y1 = (y + bh) * stride;
        for x in 0..=(width - bw) {
            let sum = sat[y1 + x + bw] - sat[y0 + x + bw] - sat[y1 + x] + sat[y0 + x];
            let mean = (sum / area).max(0.0);
            best = best.max(mean);
        }
    }
    best
}

#[expect(
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "image dimensions are far below usize::MAX / 4"
)]
fn box3x3(image: &[[f64; 4]], width: usize, height: usize) -> Vec<[f64; 4]> {
    let mut out = vec![[0.0; 4]; width * height];
    let w_i64 = width as i64;
    let h_i64 = height as i64;
    out.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
        for (x, dst) in row.iter_mut().enumerate() {
            let mut sum = [0.0; 4];
            let mut count = 0.0;
            for dy in -1_i64..=1 {
                for dx in -1_i64..=1 {
                    let yy = y as i64 + dy;
                    let xx = x as i64 + dx;
                    if yy >= 0 && yy < h_i64 && xx >= 0 && xx < w_i64 {
                        let px = image[yy as usize * width + xx as usize];
                        for (s, c) in sum.iter_mut().zip(px) {
                            *s += c;
                        }
                        count += 1.0;
                    }
                }
            }
            for s in &mut sum {
                *s /= count;
            }
            *dst = sum;
        }
    });
    out
}

fn local_error(reference: &[[f64; 4]], candidate: &[[f64; 4]]) -> f64 {
    let mut max = 0.0_f64;
    for (r, t) in reference.iter().zip(candidate) {
        for i in 0..4 {
            max = max.max((r[i] - t[i]).abs());
        }
    }
    max
}
