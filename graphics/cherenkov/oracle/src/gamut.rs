//! Gamut mapping into a `[0, 1]` destination gamut — the `f64`
//! reference for the presentation pass's out-of-gamut handling.
//!
//! The destination is a linear `r, g, b ∈ [0, 1]` gamut: linear sRGB,
//! or — for wide-gamut SDR output (#98) — linear Display P3. Everything
//! here is linear-light; the destination's transfer function applies
//! after mapping, in the caller.
//!
//! The selected algorithm is an analytic `OKLab` clip — Ottosson's
//! cusp-triangle gamut boundary with one Halley refinement, projecting
//! towards the lightness axis at adaptively-chosen lightness
//! (`ok_color.h`'s `gamut_clip_adaptive_L0_L_cusp`). It preserves hue
//! exactly and runs at fixed per-pixel cost, which is why the present
//! pass ships it rather than the CSS Color 4 binary search (#96: the
//! search's data-dependent loop was ~2.7x the analytic pass's GPU time
//! on lavapipe, and its `ΔE_OK` to the spec map stayed under one JND on
//! the sweep). `bench gamut-sweep` carries the spec algorithm as its
//! measurement reference.
//!
//! The boundary machinery is destination-general; each gamut supplies
//! its own `LMS` conversion matrices, per-channel weights and cusp
//! coefficient fits. sRGB's are `ok_color.h`'s, unchanged; Display P3's
//! are fitted against this module's `f64` boundary (root found by
//! bisection, coefficients by least squares over the sector where the
//! channel binds) — the closed fit-and-refine loop #98's plan calls
//! for.
//!
//! In-gamut colours return bit-for-bit: the caller's in-gamut fast path,
//! and the function's own bounds check, return the input untouched, so
//! an in-gamut colour's output equals plain conversion plus clamp.

use crate::color::mat3_mul;

/// A `[0, 1]` destination gamut the analytic clip targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gamut {
    /// Linear sRGB (BT.709 primaries).
    Srgb,
    /// Linear Display P3 — the wide-gamut SDR destination (#98).
    P3,
}

/// Linear sRGB → `LMS` (the `OKLab` cone-response matrix, before the
/// nonlinearity).
const SRGB_TO_LMS: [[f64; 3]; 3] = [
    [0.412_221_470_8, 0.536_332_536_3, 0.051_445_992_9],
    [0.211_903_498_2, 0.680_699_545_1, 0.107_396_956_6],
    [0.088_302_461_9, 0.281_718_837_6, 0.629_978_700_5],
];

/// Linear Display P3 → `LMS`: `XYZ_TO_LMS · P3_TO_XYZ` — the same
/// cone-response composition the sRGB matrix performs on its primaries.
const P3_TO_LMS: [[f64; 3]; 3] = [
    [
        0.481_327_291_167_537_4,
        0.462_067_911_606_910_6,
        0.056_495_602_847_820_625,
    ],
    [
        0.228_838_101_358_257_8,
        0.653_234_399_708_898_6,
        0.117_954_413_194_842_7,
    ],
    [
        0.083_986_017_799_548_08,
        0.224_272_789_264_749_47,
        0.692_220_838_877_988_7,
    ],
];

/// `cbrt(LMS)` → `OKLab`.
const LMS_TO_OKLAB: [[f64; 3]; 3] = [
    [0.210_454_255_3, 0.793_617_785_0, -0.004_072_046_8],
    [1.977_998_495_1, -2.428_592_205_0, 0.450_593_709_9],
    [0.025_904_037_1, 0.782_771_766_2, -0.808_675_766_0],
];

/// `OKLab` → `LMS′` (the pre-cube intermediates; the matrix's first column
/// is all `1` — `L` enters each channel whole).
const OKLAB_TO_LMS: [[f64; 3]; 3] = [
    [1.0, 0.396_337_777_4, 0.215_803_757_3],
    [1.0, -0.105_561_345_8, -0.063_854_172_8],
    [1.0, -0.089_484_177_5, -1.291_485_548_0],
];

/// LMS (cubed) → linear sRGB.
const LMS_TO_SRGB: [[f64; 3]; 3] = [
    [4.076_741_662_1, -3.307_711_591_3, 0.230_969_929_2],
    [-1.268_438_004_6, 2.609_757_401_1, -0.341_319_396_5],
    [-0.004_196_086_3, -0.703_418_614_7, 1.707_614_701_0],
];

/// LMS (cubed) → linear Display P3 (`inverse(P3_TO_LMS)`).
const LMS_TO_P3: [[f64; 3]; 3] = [
    [
        3.128_110_529_800_541_6,
        -2.257_075_019_413_745,
        0.129_304_788_687_192_36,
    ],
    [
        -1.091_128_161_680_977_6,
        2.413_266_763_056_220_6,
        -0.322_168_171_073_828_45,
    ],
    [
        -0.026_013_649_630_164_695,
        -0.508_027_649_102_772_6,
        1.533_316_682_253_021_8,
    ],
];

/// The per-channel data `compute_max_saturation` iterates: five
/// polynomial coefficients estimating the channel's saturation root,
/// then the channel's `LMS → RGB` weights.
///
/// sRGB's coefficients are `ok_color.h`'s. Display P3's are least-squares
/// fits over the hue sector where the channel's root is binding — the
/// same fit family, regenerated against the `f64` P3 boundary so the
/// map never reuses sRGB's cusp on a P3 destination (#98).
const SRGB_SETS: [[f64; 8]; 3] = [
    // Red, green, blue: five polynomial coefficients then the LMS→sRGB
    // channel weights.
    [
        1.190_862_77,
        1.765_767_28,
        0.596_626_41,
        0.755_151_97,
        0.567_712_45,
        4.076_741_662_1,
        -3.307_711_591_3,
        0.230_969_929_2,
    ],
    [
        0.739_565_15,
        -0.459_544_04,
        0.082_854_27,
        0.125_410_70,
        0.145_032_04,
        -1.268_438_004_6,
        2.609_757_401_1,
        -0.341_319_396_5,
    ],
    [
        1.357_336_52,
        -0.009_157_99,
        -1.151_302_10,
        -0.505_596_06,
        0.006_921_67,
        -0.004_196_086_3,
        -0.703_418_614_7,
        1.707_614_701_0,
    ],
];

const P3_SETS: [[f64; 8]; 3] = [
    [
        1.491_298_457_8,
        2.121_229_962_7,
        0.756_216_661_0,
        0.875_594_097_0,
        0.693_125_564_3,
        3.128_110_529_8,
        -2.257_075_019_4,
        0.129_304_788_7,
    ],
    [
        0.772_782_583_7,
        -0.453_392_631_1,
        0.114_116_367_4,
        0.136_965_166_7,
        -0.170_399_028_1,
        -1.091_128_161_7,
        2.413_266_763_1,
        -0.322_168_171_1,
    ],
    [
        1.519_994_485_5,
        -0.030_919_771_1,
        -1.282_327_207_0,
        -0.556_711_542_3,
        0.025_023_416_5,
        -0.026_013_649_6,
        -0.508_027_649_1,
        1.533_316_682_3,
    ],
];

/// Signed cube root — the LMS nonlinearity, safe for the negative
/// components an out-of-gamut colour produces.
fn cbrt(x: f64) -> f64 {
    x.cbrt()
}

const fn to_lms(gamut: Gamut) -> &'static [[f64; 3]; 3] {
    match gamut {
        Gamut::Srgb => &SRGB_TO_LMS,
        Gamut::P3 => &P3_TO_LMS,
    }
}

const fn from_lms(gamut: Gamut) -> &'static [[f64; 3]; 3] {
    match gamut {
        Gamut::Srgb => &LMS_TO_SRGB,
        Gamut::P3 => &LMS_TO_P3,
    }
}

const fn sets(gamut: Gamut) -> &'static [[f64; 8]; 3] {
    match gamut {
        Gamut::Srgb => &SRGB_SETS,
        Gamut::P3 => &P3_SETS,
    }
}

/// Linear `gamut` RGB (possibly outside `[0, 1]`) → `OKLab` `[L, a, b]`.
#[must_use]
pub fn rgb_to_oklab(gamut: Gamut, rgb: [f64; 3]) -> [f64; 3] {
    let lms = mat3_mul(to_lms(gamut), rgb);
    mat3_mul(&LMS_TO_OKLAB, lms.map(cbrt))
}

/// `OKLab` → linear `gamut` RGB (not clamped).
#[must_use]
pub fn oklab_to_rgb(gamut: Gamut, lab: [f64; 3]) -> [f64; 3] {
    let lms_ = mat3_mul(&OKLAB_TO_LMS, lab);
    mat3_mul(from_lms(gamut), lms_.map(|v| v * v * v))
}

/// Linear sRGB (possibly outside `[0, 1]`) → `OKLab` `[L, a, b]`.
#[must_use]
pub fn linear_srgb_to_oklab(rgb: [f64; 3]) -> [f64; 3] {
    rgb_to_oklab(Gamut::Srgb, rgb)
}

/// `OKLab` → linear sRGB (not clamped).
#[must_use]
pub fn oklab_to_linear_srgb(lab: [f64; 3]) -> [f64; 3] {
    oklab_to_rgb(Gamut::Srgb, lab)
}

/// Linear Display P3 (possibly outside `[0, 1]`) → `OKLab` `[L, a, b]`.
#[must_use]
pub fn linear_p3_to_oklab(rgb: [f64; 3]) -> [f64; 3] {
    rgb_to_oklab(Gamut::P3, rgb)
}

/// `OKLab` → linear Display P3 (not clamped).
#[must_use]
pub fn oklab_to_linear_p3(lab: [f64; 3]) -> [f64; 3] {
    oklab_to_rgb(Gamut::P3, lab)
}

/// `ΔE_OK` — Euclidean distance in `OKLab` (CSS Color 4 §20.3).
#[must_use]
pub fn delta_e_ok(one: [f64; 3], two: [f64; 3]) -> f64 {
    let [dl, da, db] = [one[0] - two[0], one[1] - two[1], one[2] - two[2]];
    db.mul_add(db, da.mul_add(da, dl * dl)).sqrt()
}

/// Whether a linear-RGB colour is inside the gamut's `[0, 1]` bounds.
fn in_gamut(rgb: [f64; 3]) -> bool {
    rgb.iter().all(|&c| (0.0..=1.0).contains(&c))
}

/// Clamps each component to `[0, 1]` — the spec's `clip`.
fn clip(rgb: [f64; 3]) -> [f64; 3] {
    rgb.map(|c| c.clamp(0.0, 1.0))
}

/// The maximum saturation `S = C/L` reachable along the `OKLab` hue
/// direction `(a, b)` (normalized) inside `gamut` — `ok_color.h`'s
/// `compute_max_saturation` structure (per-channel polynomial estimates
/// refined by one Halley step each).
fn compute_max_saturation(gamut: Gamut, a: f64, b: f64) -> f64 {
    // Max saturation is where one destination channel first goes to
    // zero. The reference implementation picks the channel by a fitted
    // partition of hue space; instead we solve all three channel
    // surfaces and take the smallest root that one Halley step actually
    // lands on (residual near zero). That is cheaper to make
    // deterministic across `f32` and `f64`: the partition conditions sit
    // within ~1e-6 of 1.0 at the sRGB vertex hues, so the two
    // precisions can pick different branches there.
    let k_l = 0.396_337_777_4f64.mul_add(a, 0.215_803_757_3 * b);
    let k_m = (-0.105_561_345_8f64).mul_add(a, -0.063_854_172_8 * b);
    let k_s = (-0.089_484_177_5f64).mul_add(a, -1.291_485_548_0 * b);

    // One Halley step from each channel's polynomial estimate; a step that
    // diverged (no real root in reach — the channel that only exceeds the
    // gamut, never zeros) leaves a large residual and is discarded.
    let eval = |s: f64, w: [f64; 3]| {
        let l_ = s.mul_add(k_l, 1.0);
        let m_ = s.mul_add(k_m, 1.0);
        let s_ = s.mul_add(k_s, 1.0);
        let l = l_ * l_ * l_;
        let m = m_ * m_ * m_;
        let sc = s_ * s_ * s_;
        let l_ds = 3.0 * k_l * l_ * l_;
        let m_ds = 3.0 * k_m * m_ * m_;
        let s_ds = 3.0 * k_s * s_ * s_;
        let l_ds2 = 6.0 * k_l * k_l * l_;
        let m_ds2 = 6.0 * k_m * k_m * m_;
        let s_ds2 = 6.0 * k_s * k_s * s_;
        (
            ws_eval(w, l, m, sc),
            ws_eval(w, l_ds, m_ds, s_ds),
            ws_eval(w, l_ds2, m_ds2, s_ds2),
        )
    };
    let mut s = f64::INFINITY;
    let mut fallback = f64::INFINITY;
    for [k0, k1, k2, k3, k4, wl, wm, ws] in *sets(gamut) {
        // Polynomial estimate of this channel's root.
        let est = k4.mul_add(a * b, k3.mul_add(a * a, b.mul_add(k2, a.mul_add(k1, k0))));
        if est > 0.0 {
            fallback = fallback.min(est);
        }
        let (f, f1, f2) = eval(est, [wl, wm, ws]);
        let den = f1.mul_add(f1, -0.5 * f * f2);
        if den == 0.0 {
            continue;
        }
        let root = est - f * f1 / den;
        // Keep the root only if the step converged onto the surface — a
        // diverged step leaves a large residual at its landing point.
        if root.is_finite() && root > 0.0 && eval(root, [wl, wm, ws]).0.abs() < 0.05 {
            s = s.min(root);
        }
    }
    if s.is_finite() { s } else { fallback.max(0.0) }
}

fn ws_eval(w: [f64; 3], l: f64, m: f64, s: f64) -> f64 {
    w[2].mul_add(s, w[0].mul_add(l, w[1] * m))
}

/// The cusp of `gamut`'s slice at hue `(a, b)` — the `(L, C)` point
/// of maximum chroma on that hue's boundary (`ok_color.h`'s
/// `find_cusp`).
fn find_cusp(gamut: Gamut, a: f64, b: f64) -> (f64, f64) {
    let s_cusp = compute_max_saturation(gamut, a, b);
    // At max saturation one channel sits at zero; scale so the largest
    // channel reaches 1 — that point is the cusp.
    let rgb = oklab_to_rgb(gamut, [1.0, s_cusp * a, s_cusp * b]);
    let l_cusp = cbrt(1.0 / rgb[0].max(rgb[1]).max(rgb[2]));
    (l_cusp, l_cusp * s_cusp)
}

/// Intersects the segment `(L0, 0) → (L1, C1)` in the `OKLCh` slice of hue
/// `(a, b)` with `gamut`'s boundary, returning the parameter `t`
/// (`t·C1` is the boundary chroma). The lower half of the boundary is
/// the cusp triangle's linear edge; the upper half takes one Halley step
/// against the true boundary surface (`ok_color.h`'s
/// `find_gamut_intersection`).
#[allow(clippy::similar_names, clippy::many_single_char_names)] // names mirror ok_color.h
fn find_gamut_intersection(
    gamut: Gamut,
    a: f64,
    b: f64,
    l1: f64,
    c1: f64,
    l0: f64,
    cusp: (f64, f64),
) -> f64 {
    let (lc, cc) = cusp;
    if (l1 - l0).mul_add(cc, -(lc - l0) * c1) <= 0.0 {
        // Lower half: the triangle's linear edge is the approximation.
        return cc * l0 / c1.mul_add(lc, cc * (l0 - l1));
    }
    // Upper half: intersect the triangle edge, then take one Halley step
    // against the true (cubic) boundary — the upper surface is curved.
    let t = cc * (l0 - 1.0) / (c1.mul_add(lc - 1.0, cc * (l0 - l1)));
    let dl = l1 - l0;
    let dc = c1;
    let k_l = 0.396_337_777_4f64.mul_add(a, 0.215_803_757_3 * b);
    let k_m = (-0.105_561_345_8f64).mul_add(a, -0.063_854_172_8 * b);
    let k_s = (-0.089_484_177_5f64).mul_add(a, -1.291_485_548_0 * b);
    let l_dt = dc.mul_add(k_l, dl);
    let m_dt = dc.mul_add(k_m, dl);
    let s_dt = dc.mul_add(k_s, dl);
    let l_at = l0.mul_add(1.0 - t, t * l1);
    let c_at = t * c1;
    let l_ = c_at.mul_add(k_l, l_at);
    let m_ = c_at.mul_add(k_m, l_at);
    let s_ = c_at.mul_add(k_s, l_at);
    let l3 = l_ * l_ * l_;
    let m3 = m_ * m_ * m_;
    let s3 = s_ * s_ * s_;
    let ldt = 3.0 * l_dt * l_ * l_;
    let mdt = 3.0 * m_dt * m_ * m_;
    let sdt = 3.0 * s_dt * s_ * s_;
    let ldt2 = 6.0 * l_dt * l_dt * l_;
    let mdt2 = 6.0 * m_dt * m_dt * m_;
    let sdt2 = 6.0 * s_dt * s_dt * s_;
    // Halley on each channel's `= 1` face; the smallest forward step wins.
    let mut step = f64::MAX;
    for w in *from_lms(gamut) {
        let r = w[0].mul_add(l3, w[1].mul_add(m3, w[2] * s3)) - 1.0;
        let r1 = w[0].mul_add(ldt, w[1].mul_add(mdt, w[2] * sdt));
        let r2 = w[0].mul_add(ldt2, w[1].mul_add(mdt2, w[2] * sdt2));
        let u = r1 / r1.mul_add(r1, -0.5 * r * r2);
        if u >= 0.0 {
            step = step.min(-r * u);
        }
    }
    t + step
}

/// The analytic `OKLab` gamut clip: `ok_color.h`'s
/// `gamut_clip_adaptive_L0_L_cusp` with `alpha = 0.05`.
///
/// Projects the out-of-gamut `rgb` towards the lightness axis in its
/// hue's `OKLCh` slice — the segment from `(L0, 0)` to the colour — and
/// returns the boundary point. `L0` blends adaptively between the
/// colour's own lightness (near the gamut) and the cusp's lightness
/// (far out), so saturated brights keep their chroma and lightness moves
/// only as far as the projection travels. Hue is preserved exactly.
/// Fixed per-pixel cost: one cusp solve, one segment intersection, one
/// Halley step on the upper half.
///
/// A final `[0, 1]` clamp absorbs the triangle approximation's residual
/// — under a thousandth of the range — so the output is always storable.
#[must_use]
pub fn gamut_map_analytic(gamut: Gamut, rgb: [f64; 3]) -> [f64; 3] {
    gamut_map_project(gamut, rgb, Anchor::Adaptive)
}

/// The analytic `OKLab` clip into linear sRGB.
#[must_use]
pub fn gamut_map_srgb_analytic(rgb: [f64; 3]) -> [f64; 3] {
    gamut_map_analytic(Gamut::Srgb, rgb)
}

/// The analytic `OKLab` clip into linear Display P3 — the wide-gamut
/// SDR destination's map (#98).
#[must_use]
pub fn gamut_map_p3_analytic(rgb: [f64; 3]) -> [f64; 3] {
    gamut_map_analytic(Gamut::P3, rgb)
}

/// Which point on the lightness axis the projection anchors at —
/// `ok_color.h`'s `L0` choices.
#[derive(Debug, Clone, Copy)]
pub enum Anchor {
    /// `clamp(L, 0, 1)` — the colour's own lightness
    /// (`gamut_clip_preserve_chroma`).
    Preserve,
    /// The hue slice's cusp lightness (`gamut_clip_project_to_L_cusp`).
    Cusp,
    /// Soft interpolation between own lightness and cusp lightness
    /// (`gamut_clip_adaptive_L0_L_cusp`, `alpha = 0.05`).
    Adaptive,
}

/// The shared projection core under [`Anchor`], into `gamut`'s `[0, 1]`
/// bounds.
#[must_use]
#[allow(clippy::many_single_char_names)] // l/a/b/c mirror OKLab notation
pub fn gamut_map_project(gamut: Gamut, rgb: [f64; 3], anchor: Anchor) -> [f64; 3] {
    if in_gamut(rgb) {
        return rgb;
    }
    let lab = rgb_to_oklab(gamut, rgb);
    // The spec's local-MINDE rule: when the plain channel-clip is already
    // within a ΔE_OK JND of the colour, keep the clip — the pre-#96 bytes.
    // Besides the perceptual headroom (the projection could only move the
    // colour a sub-just-noticeable distance), this keeps a colour that is
    // a few ULPs out of gamut — an in-gamut value round-tripped through
    // the conversion matrices — from entering the projection where the
    // cusp fit is weakest.
    let clipped = clip(rgb);
    if delta_e_ok(rgb_to_oklab(gamut, clipped), lab) < 0.02 {
        return clipped;
    }
    let [l, a, b] = lab;
    let c = (1e-5_f64).max(a.hypot(b));
    let (a_, b_) = (a / c, b / c);
    if a.hypot(b) <= 1e-5 {
        // Achromatic: no hue direction to clip along. The only way to be
        // out of gamut at zero chroma is lightness past an end — the
        // anchor is the axis and the clamp lands exactly on it.
        return clip(oklab_to_rgb(gamut, [l.clamp(0.0, 1.0), 0.0, 0.0]));
    }
    let cusp = find_cusp(gamut, a_, b_);
    let l0 = match anchor {
        Anchor::Preserve => l.clamp(0.0, 1.0),
        Anchor::Cusp => cusp.0,
        Anchor::Adaptive => {
            // Smoothly blend the anchor towards the cusp as the colour
            // recedes — `gamut_clip_adaptive_L0_L_cusp`, alpha = 0.05.
            let ld = l - cusp.0;
            let k = 2.0 * if ld > 0.0 { 1.0 - cusp.0 } else { cusp.0 };
            let e1 = 0.05f64.mul_add(c / k, 0.5f64.mul_add(k, ld.abs()));
            let root = e1.mul_add(e1, -2.0 * k * ld.abs()).sqrt();
            (0.5 * ld.signum()).mul_add(e1 - root, cusp.0)
        }
    };
    let t = find_gamut_intersection(gamut, a_, b_, l, c, l0, cusp);
    let l_out = l0.mul_add(1.0 - t, t * l);
    let c_out = t * c;
    clip(oklab_to_rgb(gamut, [l_out, c_out * a_, c_out * b_]))
}

/// The shared projection core into linear sRGB under [`Anchor`].
#[must_use]
pub fn gamut_map_srgb_project(rgb: [f64; 3], anchor: Anchor) -> [f64; 3] {
    gamut_map_project(Gamut::Srgb, rgb, anchor)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `OKLab` → `OKLCh` `[L, C, h]` with `h` in radians.
    fn oklab_to_oklch([l, a, b]: [f64; 3]) -> [f64; 3] {
        [l, a.hypot(b), b.atan2(a)]
    }

    fn assert_close3(actual: [f64; 3], expected: [f64; 3], tol: f64) {
        for (a, e) in actual.into_iter().zip(expected) {
            assert!((a - e).abs() <= tol, "{a} != {e}");
        }
    }

    #[test]
    fn oklab_round_trip() {
        for rgb in [
            [0.0; 3],
            [1.0; 3],
            [0.25, 0.5, 0.75],
            [1.0, 0.0, 0.0],
            [0.02, 0.9, 0.3],
        ] {
            // The published OKLab matrices carry f32 precision — the
            // round trip's residual is ~1e-7, far under one JND (0.02).
            assert_close3(oklab_to_linear_srgb(linear_srgb_to_oklab(rgb)), rgb, 1e-6);
        }
    }

    #[test]
    fn oklab_p3_round_trip() {
        // The P3 conversion matrices invert: linear P3 -> OKLab -> P3.
        for rgb in [[0.2, 0.6, 0.4], [1.0, 0.0, 0.0], [0.5, 0.5, 0.5]] {
            assert_close3(oklab_to_linear_p3(linear_p3_to_oklab(rgb)), rgb, 1e-6);
        }
    }

    #[test]
    fn oklab_white_is_one() {
        let [l, a, b] = linear_srgb_to_oklab([1.0, 1.0, 1.0]);
        assert!((l - 1.0).abs() < 1e-6);
        assert!(a.abs() < 1e-6 && b.abs() < 1e-6);
    }

    #[test]
    fn in_gamut_is_identity() {
        // The map returns an in-gamut colour bit-for-bit.
        for gamut in [Gamut::Srgb, Gamut::P3] {
            let colors = [
                [0.0; 3],
                [1.0; 3],
                [0.25, 0.5, 0.75],
                [0.001, 0.999, 0.5],
                [0.5, 0.5, 0.5],
            ];
            for rgb in colors {
                assert_eq!(
                    gamut_map_analytic(gamut, rgb).map(f64::to_bits),
                    rgb.map(f64::to_bits)
                );
            }
        }
    }

    #[test]
    fn out_of_gamut_lands_in_gamut() {
        // P3 red in linear sRGB has a negative green and blue and a red
        // above 1; the map lands inside [0, 1] at preserved hue.
        let out = [1.224_940_2, -0.224_940_2, 0.0];
        let mapped = gamut_map_srgb_analytic(out);
        assert!(in_gamut(mapped), "{mapped:?}");
        // Hue is preserved exactly by construction; the boundary
        let origin_h = oklab_to_oklch(linear_srgb_to_oklab(out))[2];
        let mapped_h = oklab_to_oklch(linear_srgb_to_oklab(mapped))[2];
        let dh = (mapped_h - origin_h)
            .abs()
            .rem_euclid(std::f64::consts::TAU);
        let dh = dh.min(std::f64::consts::TAU - dh);
        assert!(dh.to_degrees() < 15.0, "hue moved {} deg", dh.to_degrees());
    }

    #[test]
    fn hdr_lightness_maps_to_white_or_black() {
        // Lightness past the gamut's ends lands on the axis: 4x white
        // maps to white, a negative colour to black. The P3 conversion
        // matrices carry ~1e-3 residual at the white point, so the P3
        // bound is looser.
        for gamut in [Gamut::Srgb, Gamut::P3] {
            assert_close3(gamut_map_analytic(gamut, [4.0; 3]), [1.0; 3], 2e-3);
            assert_close3(gamut_map_analytic(gamut, [-2.0; 3]), [0.0; 3], 2e-3);
        }
    }

    #[test]
    fn p3_primaries_move_reasonably() {
        // P3 primaries (linear P3 coords -> linear sRGB, out of gamut)
        // should map to strongly saturated sRGB colours, not muddy ones.
        for p3 in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] {
            let srgb = crate::color::linear_p3_to_linear_srgb(p3);
            let mapped = gamut_map_srgb_analytic(srgb);
            assert!(in_gamut(mapped));
            let c = oklab_to_oklch(linear_srgb_to_oklab(mapped))[1];
            assert!(c > 0.1, "chroma collapsed: {mapped:?}");
        }
    }

    #[test]
    fn p3_gamut_boundary_reached_on_a_sweep() {
        // Every hue direction: a colour just past the P3 boundary maps
        // inside [0, 1] at preserved hue, never beyond it — the fitted
        // cusp coefficients' correctness bound (#98). ~1e-3 slack for the
        // one-Halley-step approximation at the triangle's edge.
        let mut worst_hue_deg = 0.0f64;
        let mut worst_out = 0.0f64;
        for i in 0..720 {
            let h = f64::from(i).to_radians() * 0.5;
            // A far-out colour on this hue: OKLab (0.5, 0.5·cos h, 0.5·sin h)
            // is deep outside every slice's boundary.
            let lab = [0.5, 0.5 * h.cos(), 0.5 * h.sin()];
            let out = oklab_to_linear_p3(lab);
            let mapped = gamut_map_p3_analytic(out);
            assert!(
                in_gamut(mapped),
                "hue {h}: out {out:?} mapped to {mapped:?}"
            );
            let origin_h = oklab_to_oklch(linear_p3_to_oklab(out))[2];
            let mapped_h = oklab_to_oklch(linear_p3_to_oklab(mapped))[2];
            let dh = (mapped_h - origin_h)
                .abs()
                .rem_euclid(std::f64::consts::TAU);
            let dh = dh.min(std::f64::consts::TAU - dh).to_degrees();
            worst_hue_deg = worst_hue_deg.max(dh);
            worst_out = worst_out.max(mapped.iter().map(|&c| c - 1.0).fold(0.0, f64::max));
        }
        assert!(worst_hue_deg < 1.0, "hue moved {worst_hue_deg} deg");
        assert!(worst_out < 1e-3, "boundary overshoot {worst_out}");
    }
}
