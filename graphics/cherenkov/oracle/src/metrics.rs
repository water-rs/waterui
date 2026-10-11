//! Error metrics: FLIP (and HDR-FLIP when channel values exceed `1.0`), plus
//! maximum local error.
//!
//! FLIP is ported from `FLIP.h` in <https://github.com/NVlabs/flip>
//! (BSD-3-Clause) — the published algorithm of
//!
//! - Andersson, Nilsson, Akenine-Möller, Oskarsson, Åström, Fairchild.
//!   "FLIP: A Difference Evaluator for Alternating Images", HPG 2020.
//! - Andersson, Nilsson, Åström, Oskarsson. "HDR-FLIP", 2021.
//!
//! Inputs are linear-light RGB in Display P3 primaries — the working space
//! itself (premultiplied RGBA: premultiplied RGB is the colour over black,
//! which is what an opaque-surface comparison sees). FLIP's XYZ step uses
//! the P3→XYZ matrix, so colours outside the sRGB gamut are measured rather
//! than clipped into it; `cmax` is the P3 green–blue `HyAB` distance.
//!
//! PPD uses the reference default: viewing distance 0.7 m, screen width
//! 3840 px, monitor width 0.7 m → `0.7 * (3840 / 0.7) * π/180 ≈ 67.0`.

use std::f64::consts::PI;

use crate::color::{P3_TO_XYZ, mat3_inv, mat3_mul};
use crate::image::F32Image;

/// The default pixels-per-degree of the reference implementation.
pub const DEFAULT_PPD: f64 = 0.7 * (3840.0 / 0.7) * (PI / 180.0);

const GQC: f64 = 0.7;
const GPC: f64 = 0.4;
const GPT: f64 = 0.95;
const GW: f64 = 0.082;

const A1: [f64; 3] = [1.0, 1.0, 34.1];
const B1: [f64; 3] = [0.0047, 0.0053, 0.04];
const A2: [f64; 3] = [0.0, 0.0, 13.5];
const B2: [f64; 3] = [1.0e-5, 1.0e-5, 0.025];

#[allow(clippy::unreadable_literal)] // verbatim reference constants
const INV_ILLUMINANT: [f64; 3] = [1.052156925, 1.0, 0.918357670];
#[allow(clippy::unreadable_literal)]
const ILLUMINANT: [f64; 3] = [0.950428545, 1.0, 1.088900371];

/// ACES tone-mapping coefficients (index 1 of `ToneMappingCoefficients`).
const ACES: [f64; 6] = [0.9036, 0.018, 0.0, 0.8748, 0.354, 0.14];

fn xyz_to_ycxcz(xyz: [f64; 3]) -> [f64; 3] {
    let x = xyz[0] * INV_ILLUMINANT[0];
    let y = xyz[1] * INV_ILLUMINANT[1];
    let z = xyz[2] * INV_ILLUMINANT[2];
    [116.0f64.mul_add(y, -16.0), 500.0 * (x - y), 200.0 * (y - z)]
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
        116.0f64.mul_add(v[1], -16.0),
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

/// Compute `cmax` (maximum `HyAB` distance, green vs blue, `gqc`-powered).
fn compute_max_distance() -> f64 {
    let lab = |rgb: [f64; 3]| xyz_to_cielab(mat3_mul(&P3_TO_XYZ, rgb));
    let g = lab([0.0, 1.0, 0.0]);
    let b = lab([0.0, 0.0, 1.0]);
    let gh = [g[0], hunt(g[0], g[1]), hunt(g[0], g[2])];
    let bh = [b[0], hunt(b[0], b[1]), hunt(b[0], b[2])];
    hyab(gh, bh).powf(GQC)
}

/// The `(Y, Cx)` and `(Cz1, Cz2)` spatial filter weights, `2*radius+1` taps.
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

/// `(Gaussian, 1st derivative, 2nd derivative)` feature filter weights.
#[allow(clippy::similar_names)] // names mirror the reference implementation
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

/// Separable spatial filtering + colour difference per pixel (kernel steps
/// of `LDR_FLIP` in the reference). `ref`/`test` are linear RGB `w*h*3`.
#[allow(clippy::too_many_arguments)] // signature mirrors the reference kernels
fn color_difference(
    reference: &[[f64; 3]],
    test: &[[f64; 3]],
    width: usize,
    height: usize,
    fycx: &[[f64; 2]],
    fcz: &[[f64; 2]],
    cmax: f64,
    pccmax: f64,
) -> Vec<f64> {
    let radius = fycx.len() / 2;
    let xyz_to_p3 = mat3_inv(&P3_TO_XYZ);
    // Step 1: YCxCz conversion.
    let to_ycxcz = |img: &[[f64; 3]]| -> Vec<[f64; 3]> {
        img.iter()
            .map(|rgb| xyz_to_ycxcz(mat3_mul(&P3_TO_XYZ, *rgb)))
            .collect()
    };
    let ref_ycxcz = to_ycxcz(reference);
    let test_ycxcz = to_ycxcz(test);

    // Step 2: spatial filter in x — YCx intermediate + Cz pair.
    let filter_x = |img: &[[f64; 3]]| -> (Vec<[f64; 2]>, Vec<[f64; 2]>) {
        let mut ycx = vec![[0.0; 2]; img.len()];
        let mut cz = vec![[0.0; 2]; img.len()];
        for y in 0..height {
            for x in 0..width {
                let (mut sycx, mut scz) = ([0.0; 2], [0.0; 2]);
                for (k, &[wy, wc]) in fycx.iter().enumerate() {
                    let xx = (x + k).saturating_sub(radius).min(width - 1);
                    let c = img[y * width + xx];
                    sycx[0] = wy.mul_add(c[0], sycx[0]);
                    sycx[1] = wc.mul_add(c[1], sycx[1]);
                    scz[0] = fcz[k][0].mul_add(c[2], scz[0]);
                    scz[1] = fcz[k][1].mul_add(c[2], scz[1]);
                }
                ycx[y * width + x] = sycx;
                cz[y * width + x] = scz;
            }
        }
        (ycx, cz)
    };
    let (ref_ycx, ref_cz) = filter_x(&ref_ycxcz);
    let (test_ycx, test_cz) = filter_x(&test_ycxcz);

    // Step 3: filter in y, then LabHunt difference.
    let mut diff = vec![0.0; reference.len()];
    for y in 0..height {
        for x in 0..width {
            let filtered = |ycx: &[[f64; 2]], cz: &[[f64; 2]]| -> [f64; 3] {
                let (mut sycx, mut scz) = ([0.0; 2], [0.0; 2]);
                for (k, &[wy, wc]) in fycx.iter().enumerate() {
                    let yy = (y + k).saturating_sub(radius).min(height - 1);
                    sycx[0] = wy.mul_add(ycx[yy * width + x][0], sycx[0]);
                    sycx[1] = wc.mul_add(ycx[yy * width + x][1], sycx[1]);
                    scz[0] = fcz[k][0].mul_add(cz[yy * width + x][0], scz[0]);
                    scz[1] = fcz[k][1].mul_add(cz[yy * width + x][1], scz[1]);
                }
                [sycx[0], sycx[1], scz[0] + scz[1]]
            };
            let rgb_r = mat3_mul(&xyz_to_p3, ycxcz_to_xyz(filtered(&ref_ycx, &ref_cz)))
                .map(|v| v.clamp(0.0, 1.0));
            let rgb_t = mat3_mul(&xyz_to_p3, ycxcz_to_xyz(filtered(&test_ycx, &test_cz)))
                .map(|v| v.clamp(0.0, 1.0));
            let lab_r = xyz_to_cielab(mat3_mul(&P3_TO_XYZ, rgb_r));
            let lab_t = xyz_to_cielab(mat3_mul(&P3_TO_XYZ, rgb_t));
            let hr = [lab_r[0], hunt(lab_r[0], lab_r[1]), hunt(lab_r[0], lab_r[2])];
            let ht = [lab_t[0], hunt(lab_t[0], lab_t[1]), hunt(lab_t[0], lab_t[2])];
            let mut cd = hyab(hr, ht).powf(GQC);
            cd = if cd < pccmax {
                cd * (GPT / pccmax)
            } else {
                ((cd - pccmax) / (cmax - pccmax)).mul_add(1.0 - GPT, GPT)
            };
            diff[y * width + x] = cd;
        }
    }
    diff
}

/// Feature (edge/point) detection and the final FLIP error map.
fn feature_difference(
    ref_ycxcz_y: &[f64],
    test_ycxcz_y: &[f64],
    width: usize,
    height: usize,
    filter: &[[f64; 3]],
) -> Vec<f64> {
    let radius = filter.len() / 2;
    // First direction: 1st & 2nd x-derivatives and Gaussian, on Y normalized
    // to [0,1].
    let pass1 = |img: &[f64]| -> Vec<[f64; 3]> {
        let mut out = vec![[0.0; 3]; img.len()];
        for y in 0..height {
            for x in 0..width {
                let (mut dx, mut ddx, mut g) = (0.0, 0.0, 0.0);
                for (k, w) in filter.iter().enumerate() {
                    let xx = (x + k).saturating_sub(radius).min(width - 1);
                    let yn = img[y * width + xx] / 116.0 + 16.0 / 116.0;
                    dx = w[1].mul_add(yn, dx);
                    ddx = w[2].mul_add(yn, ddx);
                    g = w[0].mul_add(yn, g);
                }
                out[y * width + x] = [dx, ddx, g];
            }
        }
        out
    };
    let ir = pass1(ref_ycxcz_y);
    let it = pass1(test_ycxcz_y);

    let mut out = vec![0.0; ref_ycxcz_y.len()];
    for y in 0..height {
        for x in 0..width {
            let mut r = [0.0; 4]; // dx, ddx, dy, ddy
            let mut t = [0.0; 4];
            for (k, w) in filter.iter().enumerate() {
                let yy = (y + k).saturating_sub(radius).min(height - 1);
                let pr = ir[yy * width + x];
                let pt = it[yy * width + x];
                r[0] = w[0].mul_add(pr[0], r[0]);
                r[1] = w[0].mul_add(pr[1], r[1]);
                r[2] = w[1].mul_add(pr[2], r[2]);
                r[3] = w[2].mul_add(pr[2], r[3]);
                t[0] = w[0].mul_add(pt[0], t[0]);
                t[1] = w[0].mul_add(pt[1], t[1]);
                t[2] = w[1].mul_add(pt[2], t[2]);
                t[3] = w[2].mul_add(pt[2], t[3]);
            }
            let edge_diff = (r[0].hypot(r[2]) - t[0].hypot(t[2])).abs();
            let point_diff = (r[1].hypot(r[3]) - t[1].hypot(t[3])).abs();
            // pow(featureDiff, gqf) with gqf = 0.5 — a square root.
            out[y * width + x] =
                (edge_diff.max(point_diff) * core::f64::consts::FRAC_1_SQRT_2).sqrt();
        }
    }
    out
}

/// LDR FLIP error map between two linear-P3 images (`w*h` RGB pixels,
/// values in the P3 gamut `[0,1]`). Returns per-pixel error in `[0,1]`.
#[must_use]
pub fn ldr_flip(
    reference: &[[f64; 3]],
    test: &[[f64; 3]],
    width: usize,
    height: usize,
    ppd: f64,
) -> Vec<f64> {
    let clamped = |img: &[[f64; 3]]| -> Vec<[f64; 3]> {
        img.iter().map(|c| c.map(|v| v.clamp(0.0, 1.0))).collect()
    };
    let r = clamped(reference);
    let t = clamped(test);
    let cmax = compute_max_distance();
    let pccmax = GPC * cmax;
    let (fycx, fcz) = spatial_filters(ppd);
    let cd = color_difference(&r, &t, width, height, &fycx, &fcz, cmax, pccmax);
    let to_y = |img: &[[f64; 3]]| -> Vec<f64> {
        img.iter()
            .map(|rgb| xyz_to_ycxcz(mat3_mul(&P3_TO_XYZ, *rgb))[0])
            .collect()
    };
    let fd = feature_difference(&to_y(&r), &to_y(&t), width, height, &feature_filter(ppd));
    (0..cd.len()).map(|i| cd[i].powf(1.0 - fd[i])).collect()
}

/// ACES tone map (HDR-FLIP's default tone mapper).
fn tone_map_aces(v: f64) -> f64 {
    (v.mul_add(ACES[1], v * v * ACES[0]) + ACES[2])
        / (v.mul_add(ACES[4], v * v * ACES[3]) + ACES[5])
}

/// Fixed HDR-FLIP exposure configuration, in EV (log2 stops relative to
/// SDR white at `1.0`).
///
/// Every scene is compared under the same exposure ladder so metrics are
/// comparable across scenes — the published HDR-FLIP algorithm's adaptive
/// per-image range would make `flip_mean` values incomparable. `0 EV` is
/// the SDR exposure; `+4 EV` maps the corpus's brightest values (16× SDR
/// white) to the tone-mapped range; five stops give one exposure per EV.
pub const HDR_EXPOSURES: [f64; 5] = [0.0, 1.0, 2.0, 3.0, 4.0];

/// HDR-FLIP error map: per-exposure LDR-FLIP max over the fixed
/// [`HDR_EXPOSURES`] ladder (`0..=+4 EV`).
#[must_use]
pub fn hdr_flip(
    reference: &[[f64; 3]],
    test: &[[f64; 3]],
    width: usize,
    height: usize,
    ppd: f64,
) -> Vec<f64> {
    let mut out = vec![0.0f64; reference.len()];
    for &exposure in &HDR_EXPOSURES {
        let scale = exposure.exp2();
        let at_exposure = |img: &[[f64; 3]]| -> Vec<[f64; 3]> {
            img.iter()
                .map(|c| c.map(|v| tone_map_aces(v * scale).clamp(0.0, 1.0)))
                .collect()
        };
        let err = ldr_flip(
            &at_exposure(reference),
            &at_exposure(test),
            width,
            height,
            ppd,
        );
        for (o, e) in out.iter_mut().zip(err) {
            *o = o.max(e);
        }
    }
    out
}

/// Maximum local error: max over pixels and channels of
/// `|box3x3(ref) − box3x3(test)|` — the images are filtered first, then
/// differenced (operating on the premultiplied RGBA channels).
#[must_use]
#[expect(
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "image dimensions are far below usize::MAX/4"
)]
pub fn max_local_error(reference: &F32Image, test: &F32Image) -> f64 {
    let (w, h) = (reference.width as usize, reference.height as usize);
    let box3x3 = |pixels: &[[f32; 4]]| -> Vec<[f64; 4]> {
        let mut out = vec![[0.0; 4]; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut sum = [0.0; 4];
                let mut n = 0.0;
                for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        let (yy, xx) = (y as i64 + dy, x as i64 + dx);
                        if yy >= 0 && yy < h as i64 && xx >= 0 && xx < w as i64 {
                            for i in 0..4 {
                                sum[i] += f64::from(pixels[yy as usize * w + xx as usize][i]);
                            }
                            n += 1.0;
                        }
                    }
                }
                for s in &mut sum {
                    *s /= n;
                }
                out[y * w + x] = sum;
            }
        }
        out
    };
    let ref_f = box3x3(&reference.pixels);
    let test_f = box3x3(&test.pixels);
    let mut max = 0.0f64;
    for (r, t) in ref_f.iter().zip(&test_f) {
        for i in 0..4 {
            max = max.max((r[i] - t[i]).abs());
        }
    }
    max
}

/// Extract a premultiplied linear-P3 `f32` image's RGB as `f64` for FLIP
/// (premultiplied = over-black comparison).
#[must_use]
pub fn to_linear_p3(img: &F32Image) -> Vec<[f64; 3]> {
    img.pixels
        .iter()
        .map(|p| [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])])
        .collect()
}

/// Whether `img` has any HDR content (channel > 1.0 in linear P3; alpha
/// excluded).
#[must_use]
pub fn is_hdr(img: &[[f64; 3]]) -> bool {
    img.iter().any(|c| c.iter().any(|v| *v > 1.0))
}

/// The 256-entry Magma colormap, from `MapMagma` in `FLIP.h`.
/// [`heat_map`] writes the FLIP error image in this colormap.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "error and magma channels are clamped to [0,1] before the u8 quantize"
)]
pub fn heat_map(error: &[f64]) -> Vec<[u8; 3]> {
    error
        .iter()
        .map(|&e| {
            let idx = (e.clamp(0.0, 1.0) * 255.0).round() as usize;
            let c = MAGMA[idx.min(255)];
            [
                (c[0] * 255.0).round() as u8,
                (c[1] * 255.0).round() as u8,
                (c[2] * 255.0).round() as u8,
            ]
        })
        .collect()
}

/// The per-scene metric report.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Metrics {
    /// Mean FLIP (or HDR-FLIP) error across pixels, `0.0..=1.0`.
    pub flip_mean: f64,
    /// Maximum per-pixel FLIP error.
    pub flip_max: f64,
    /// Maximum local error (3×3 box-filtered per-channel abs difference).
    pub max_local_error: f64,
    /// Whether HDR-FLIP was used (reference had values > 1.0).
    pub hdr: bool,
    /// HDR-FLIP's fixed exposure ladder, in EV — `0` (SDR white maps to
    /// `1.0`) through `+4` (16× SDR white), five stops. Recorded so every
    /// metrics JSON is self-describing; see [`HDR_EXPOSURES`].
    pub hdr_flip_exposure_ev: Vec<f64>,
}

/// Compare a test render against the oracle reference, computing FLIP (or
/// HDR-FLIP) and max local error, and returning the heatmap pixels.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "pixel counts are far below 2^53"
)]
pub fn compare(reference: &F32Image, test: &F32Image) -> (Metrics, Vec<u8>) {
    let (w, h) = (reference.width as usize, reference.height as usize);
    let ref_rgb = to_linear_p3(reference);
    let test_rgb = to_linear_p3(test);
    let hdr = is_hdr(&ref_rgb);
    let error = if hdr {
        hdr_flip(&ref_rgb, &test_rgb, w, h, DEFAULT_PPD)
    } else {
        ldr_flip(&ref_rgb, &test_rgb, w, h, DEFAULT_PPD)
    };
    let mut sum = 0.0;
    let mut max = 0.0f64;
    for &e in &error {
        sum += e;
        max = max.max(e);
    }
    let mle = max_local_error(reference, test);
    let heat = heat_map(&error);
    let mut rgb8 = Vec::with_capacity(heat.len() * 3);
    for c in heat {
        rgb8.extend_from_slice(&c);
    }
    (
        Metrics {
            flip_mean: if error.is_empty() {
                0.0
            } else {
                sum / error.len() as f64
            },
            flip_max: max,
            max_local_error: mle,
            hdr,
            hdr_flip_exposure_ev: HDR_EXPOSURES.to_vec(),
        },
        rgb8,
    )
}

/// Write a magma-mapped FLIP error heatmap to `path` as PNG.
///
/// # Errors
/// `std::io::Error` on encode/write failure.
pub fn write_heatmap(
    error_rgb8: &[u8],
    width: u32,
    height: u32,
    path: &std::path::Path,
) -> std::io::Result<()> {
    let file = std::fs::File::create(path)?;
    let mut writer = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(&mut writer, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut w = encoder.write_header()?;
    w.write_image_data(error_rgb8)?;
    Ok(())
}

// The 256-entry magma LUT (from `FLIP.h` MapMagma, BSD-3-Clause NVlabs/flip).
#[allow(clippy::unreadable_literal, reason = "verbatim LUT from FLIP.h")]
const MAGMA: [[f64; 3]; 256] = include!("magma_table.rs");
