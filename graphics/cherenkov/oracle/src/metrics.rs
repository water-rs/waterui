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

use rayon::prelude::*;

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

/// The filters and colour-distance constants FLIP applies at one
/// pixels-per-degree.
#[derive(Debug)]
struct Filters {
    /// `(Y, Cx)` spatial weights, `2*radius+1` taps.
    ycx: Vec<[f64; 2]>,
    /// `(Cz1, Cz2)` spatial weights, as many taps as `ycx`.
    cz: Vec<[f64; 2]>,
    /// `(Gaussian, 1st derivative, 2nd derivative)` feature weights.
    feature: Vec<[f64; 3]>,
    xyz_to_p3: [[f64; 3]; 3],
    cmax: f64,
    pccmax: f64,
}

impl Filters {
    fn new(ppd: f64) -> Self {
        let (ycx, cz) = spatial_filters(ppd);
        let cmax = compute_max_distance();
        Self {
            ycx,
            cz,
            feature: feature_filter(ppd),
            xyz_to_p3: mat3_inv(&P3_TO_XYZ),
            cmax,
            pccmax: GPC * cmax,
        }
    }
}

/// What FLIP compares of one `[0, 1]` linear-P3 image: per pixel, the
/// spatially filtered colour in Hunt-adjusted CIELAB, and the edge and
/// point feature magnitudes of its luminance. Both depend on that image
/// alone, so a reference's are computed once for every test compared
/// against it.
#[derive(Debug)]
struct Features {
    hunt: Vec<[f64; 3]>,
    edge_point: Vec<[f64; 2]>,
}

impl Features {
    fn new(rgb: &[[f64; 3]], width: usize, height: usize, filters: &Filters) -> Self {
        let ycxcz: Vec<[f64; 3]> = rgb
            .par_iter()
            .map(|rgb| xyz_to_ycxcz(mat3_mul(&P3_TO_XYZ, *rgb)))
            .collect();
        Self {
            hunt: filtered_hunt(&ycxcz, width, height, filters),
            edge_point: edge_point(&ycxcz, width, height, &filters.feature),
        }
    }
}

/// Separable spatial filtering of a `YCxCz` image, back to linear P3 and then
/// to Hunt-adjusted CIELAB (the per-image kernel steps of `LDR_FLIP` in the
/// reference).
fn filtered_hunt(
    ycxcz: &[[f64; 3]],
    width: usize,
    height: usize,
    filters: &Filters,
) -> Vec<[f64; 3]> {
    let radius = filters.ycx.len() / 2;
    let taps = || filters.ycx.iter().zip(&filters.cz).enumerate();
    // Filter in x: YCx intermediate + Cz pair.
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
    // Filter in y, then convert.
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
                    &filters.xyz_to_p3,
                    ycxcz_to_xyz([sycx[0], sycx[1], scz[0] + scz[1]]),
                )
                .map(|v| v.clamp(0.0, 1.0));
                let lab = xyz_to_cielab(mat3_mul(&P3_TO_XYZ, rgb));
                *out = [lab[0], hunt(lab[0], lab[1]), hunt(lab[0], lab[2])];
            }
        });
    hunted
}

/// Edge and point feature magnitudes of a `YCxCz` image's luminance,
/// normalized to `[0, 1]`.
fn edge_point(
    ycxcz: &[[f64; 3]],
    width: usize,
    height: usize,
    filter: &[[f64; 3]],
) -> Vec<[f64; 2]> {
    let radius = filter.len() / 2;
    // First direction: 1st & 2nd x-derivatives and Gaussian.
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
                let mut d = [0.0; 4]; // dx, ddx, dy, ddy
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

/// The per-pixel FLIP error between two images' features.
fn flip_error(reference: &Features, test: &Features, filters: &Filters) -> Vec<f64> {
    let (cmax, pccmax) = (filters.cmax, filters.pccmax);
    reference
        .hunt
        .par_iter()
        .zip(&test.hunt)
        .zip(reference.edge_point.par_iter().zip(&test.edge_point))
        .map(|((hr, ht), (fr, ft))| {
            let mut cd = hyab(*hr, *ht).powf(GQC);
            cd = if cd < pccmax {
                cd * (GPT / pccmax)
            } else {
                ((cd - pccmax) / (cmax - pccmax)).mul_add(1.0 - GPT, GPT)
            };
            let edge_diff = (fr[0] - ft[0]).abs();
            let point_diff = (fr[1] - ft[1]).abs();
            // pow(featureDiff, gqf) with gqf = 0.5 — a square root.
            let fd = (edge_diff.max(point_diff) * core::f64::consts::FRAC_1_SQRT_2).sqrt();
            cd.powf(1.0 - fd)
        })
        .collect()
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

/// The `[0, 1]` image LDR-FLIP measures: the image clamped for LDR-FLIP,
/// or tone-mapped at `exposure` EV for one stop of HDR-FLIP.
fn unit_range(rgb: &[[f64; 3]], exposure: Option<f64>) -> Vec<[f64; 3]> {
    exposure.map_or_else(
        || {
            rgb.par_iter()
                .map(|c| c.map(|v| v.clamp(0.0, 1.0)))
                .collect()
        },
        |exposure| {
            let scale = exposure.exp2();
            rgb.par_iter()
                .map(|c| c.map(|v| tone_map_aces(v * scale).clamp(0.0, 1.0)))
                .collect()
        },
    )
}

/// A reference image prepared for FLIP: its features at every exposure the
/// comparison measures — one for LDR-FLIP, one per [`HDR_EXPOSURES`] stop
/// for HDR-FLIP.
#[derive(Debug)]
struct FlipReference {
    width: usize,
    height: usize,
    hdr: bool,
    filters: Filters,
    features: Vec<Features>,
}

impl FlipReference {
    fn new(reference: &[[f64; 3]], width: usize, height: usize, ppd: f64, hdr: bool) -> Self {
        let filters = Filters::new(ppd);
        let features = Self::exposures(hdr)
            .map(|exposure| {
                Features::new(&unit_range(reference, exposure), width, height, &filters)
            })
            .collect();
        Self {
            width,
            height,
            hdr,
            filters,
            features,
        }
    }

    fn exposures(hdr: bool) -> impl Iterator<Item = Option<f64>> {
        let ladder: &[f64] = if hdr { &HDR_EXPOSURES } else { &[] };
        ladder
            .iter()
            .copied()
            .map(Some)
            .chain((!hdr).then_some(None))
    }

    /// The per-pixel error map of `test` against this reference: LDR-FLIP,
    /// or HDR-FLIP's per-exposure maximum.
    fn error(&self, test: &[[f64; 3]]) -> Vec<f64> {
        let mut maps =
            Self::exposures(self.hdr)
                .zip(&self.features)
                .map(|(exposure, reference)| {
                    let test = Features::new(
                        &unit_range(test, exposure),
                        self.width,
                        self.height,
                        &self.filters,
                    );
                    flip_error(reference, &test, &self.filters)
                });
        if !self.hdr {
            return maps.next().expect("LDR-FLIP measures one exposure");
        }
        let mut out = vec![0.0f64; test.len()];
        for err in maps {
            for (o, e) in out.iter_mut().zip(err) {
                *o = o.max(e);
            }
        }
        out
    }
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
    FlipReference::new(reference, width, height, ppd, false).error(test)
}

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
    FlipReference::new(reference, width, height, ppd, true).error(test)
}

/// An image's 3×3 box filter, per premultiplied RGBA channel, over the
/// neighbours inside the image.
#[expect(
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "image dimensions are far below usize::MAX/4"
)]
fn box3x3(image: &F32Image) -> Vec<[f64; 4]> {
    let (w, h) = (image.width as usize, image.height as usize);
    let mut out = vec![[0.0; 4]; w * h];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, out) in row.iter_mut().enumerate() {
            let mut sum = [0.0; 4];
            let mut n = 0.0;
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let (yy, xx) = (y as i64 + dy, x as i64 + dx);
                    if yy >= 0 && yy < h as i64 && xx >= 0 && xx < w as i64 {
                        let pixel = image.pixels[yy as usize * w + xx as usize];
                        for (s, c) in sum.iter_mut().zip(pixel) {
                            *s += f64::from(c);
                        }
                        n += 1.0;
                    }
                }
            }
            for s in &mut sum {
                *s /= n;
            }
            *out = sum;
        }
    });
    out
}

/// The largest per-channel difference between two box-filtered images.
fn local_error(reference: &[[f64; 4]], test: &[[f64; 4]]) -> f64 {
    let mut max = 0.0f64;
    for (r, t) in reference.iter().zip(test) {
        for i in 0..4 {
            max = max.max((r[i] - t[i]).abs());
        }
    }
    max
}

/// Maximum local error: max over pixels and channels of
/// `|box3x3(ref) − box3x3(test)|` — the images are filtered first, then
/// differenced (operating on the premultiplied RGBA channels).
#[must_use]
pub fn max_local_error(reference: &F32Image, test: &F32Image) -> f64 {
    local_error(&box3x3(reference), &box3x3(test))
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

/// An oracle reference prepared for comparison.
///
/// Everything [`compare`] derives from the reference alone — its FLIP
/// features at every exposure and its box-filtered channels — is computed
/// once, so comparing several renders against one reference pays for it
/// once.
#[derive(Debug)]
pub struct Reference {
    flip: FlipReference,
    local: Vec<[f64; 4]>,
}

impl Reference {
    /// Prepare `reference`, compared with HDR-FLIP when it has values above
    /// `1.0` and with LDR-FLIP otherwise.
    #[must_use]
    pub fn new(reference: &F32Image) -> Self {
        let rgb = to_linear_p3(reference);
        let hdr = is_hdr(&rgb);
        Self {
            flip: FlipReference::new(
                &rgb,
                reference.width as usize,
                reference.height as usize,
                DEFAULT_PPD,
                hdr,
            ),
            local: box3x3(reference),
        }
    }

    /// Compare a test render against this reference, computing FLIP (or
    /// HDR-FLIP) and max local error, and returning the heatmap pixels.
    ///
    /// # Panics
    /// When `test`'s dimensions differ from the reference's.
    #[must_use]
    pub fn compare(&self, test: &F32Image) -> (Metrics, Vec<u8>) {
        let (metrics, error) = self.evaluate(test);
        let heat = heat_map(&error);
        let mut rgb8 = Vec::with_capacity(heat.len() * 3);
        for c in heat {
            rgb8.extend_from_slice(&c);
        }
        (metrics, rgb8)
    }

    /// The metrics of [`Reference::compare`], without the heatmap.
    ///
    /// # Panics
    /// When `test`'s dimensions differ from the reference's.
    #[must_use]
    pub fn metrics(&self, test: &F32Image) -> Metrics {
        self.evaluate(test).0
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "pixel counts are far below 2^53"
    )]
    fn evaluate(&self, test: &F32Image) -> (Metrics, Vec<f64>) {
        assert_eq!(
            (self.flip.width, self.flip.height),
            (test.width as usize, test.height as usize),
            "test and reference dimensions differ"
        );
        let error = self.flip.error(&to_linear_p3(test));
        let mut sum = 0.0;
        let mut max = 0.0f64;
        for &e in &error {
            sum += e;
            max = max.max(e);
        }
        let metrics = Metrics {
            flip_mean: if error.is_empty() {
                0.0
            } else {
                sum / error.len() as f64
            },
            flip_max: max,
            max_local_error: local_error(&self.local, &box3x3(test)),
            hdr: self.flip.hdr,
            hdr_flip_exposure_ev: HDR_EXPOSURES.to_vec(),
        };
        (metrics, error)
    }
}

/// Compare a test render against the oracle reference, computing FLIP (or
/// HDR-FLIP) and max local error, and returning the heatmap pixels.
///
/// Comparing several renders against one reference goes through
/// [`Reference`], which prepares the reference once.
///
/// # Panics
/// When the images' dimensions differ.
#[must_use]
pub fn compare(reference: &F32Image, test: &F32Image) -> (Metrics, Vec<u8>) {
    Reference::new(reference).compare(test)
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

/// `flip_mean` ceiling of [`assert_images_match`]: the mean FLIP error two
/// routes to the same pixels may show.
pub const FLIP_MEAN_LIMIT: f64 = 0.006;

/// `flip_max` ceiling of [`assert_images_match`]: the worst per-pixel FLIP
/// error two routes to the same pixels may show.
pub const FLIP_MAX_LIMIT: f64 = 0.15;

/// `max_local_error` ceiling of [`assert_images_match`]: the worst 3×3
/// box-filtered per-channel difference two routes to the same pixels may show.
pub const LOCAL_ERROR_LIMIT: f64 = 0.09;

/// The per-value bound scale of [`assert_close`]:
/// `|a - e| <= CLOSE_TOLERANCE * max(1, |e|)` — `2^-12`, relative above one
/// in magnitude and absolute below.
pub const CLOSE_TOLERANCE: f64 = 1.0 / 4096.0;

/// Assert two renders of the same scene are visually equivalent.
///
/// Runs [`compare`] and panics unless `flip_mean <= FLIP_MEAN_LIMIT`,
/// `flip_max <= FLIP_MAX_LIMIT` and `max_local_error <= LOCAL_ERROR_LIMIT`.
/// On failure the message names all three metrics and the FLIP heat map is
/// written to a temp path the message prints.
///
/// # Panics
/// When the images' dimensions differ or `compare`'s metrics exceed the
/// ceilings above.
#[track_caller]
pub fn assert_images_match(reference: &F32Image, test: &F32Image) {
    assert_eq!(
        (reference.width, reference.height),
        (test.width, test.height),
        "image dimensions differ"
    );
    let (m, heat) = compare(reference, test);
    if m.flip_mean <= FLIP_MEAN_LIMIT
        && m.flip_max <= FLIP_MAX_LIMIT
        && m.max_local_error <= LOCAL_ERROR_LIMIT
    {
        return;
    }
    let caller = std::panic::Location::caller();
    let file = caller.file().rsplit(['/', '\\']).next().unwrap_or("image");
    let path = std::env::temp_dir().join(format!("cherenkov-flip-{file}-{}.png", caller.line()));
    let note = match write_heatmap(&heat, reference.width, reference.height, &path) {
        Ok(()) => format!("heat map written to {}", path.display()),
        Err(e) => format!("could not write the heat map to {}: {e}", path.display()),
    };
    panic!(
        "images differ: flip_mean {} (limit {FLIP_MEAN_LIMIT}), flip_max {} \
         (limit {FLIP_MAX_LIMIT}), max_local_error {} (limit \
         {LOCAL_ERROR_LIMIT}); {note}",
        m.flip_mean, m.flip_max, m.max_local_error
    );
}

/// Assert two kernel outputs too small to compare as images are visually
/// equivalent.
///
/// Every element `i` must satisfy
/// `|actual[i] - expected[i]| <= CLOSE_TOLERANCE * max(1, |expected[i]|)`,
/// a `2^-12` bound that is relative above one in magnitude and absolute
/// below.
///
/// # Panics
/// When the slices' lengths differ, or at the first element outside the
/// bound — the message names its index, both values and the bound.
#[track_caller]
pub fn assert_close<T: Copy + Into<f64>>(actual: &[T], expected: &[T]) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "slice lengths differ: {} actual vs {} expected",
        actual.len(),
        expected.len()
    );
    for (i, (&a, &e)) in actual.iter().zip(expected).enumerate() {
        let (a, e) = (a.into(), e.into());
        let bound = CLOSE_TOLERANCE * e.abs().max(1.0);
        assert!(
            (a - e).abs() <= bound,
            "index {i}: |{a} - {e}| exceeds {bound}"
        );
    }
}

// The 256-entry magma LUT (from `FLIP.h` MapMagma, BSD-3-Clause NVlabs/flip).
#[allow(clippy::unreadable_literal, reason = "verbatim LUT from FLIP.h")]
const MAGMA: [[f64; 3]; 256] = include!("magma_table.rs");
