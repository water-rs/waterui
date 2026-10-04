//! Presentation of the working-space framebuffer into an sRGB bitmap —
//! the CPU twin of `gpu/src/render/present.wgsl` (#96): unpremultiply,
//! the P3 → sRGB matrix, the headroom tone map (#97), the analytic
//! `OKLab` gamut map, the sRGB transfer, re-premultiply, then unorm-8.
//! In-gamut pixels produce the pre-#96 convert-and-clamp bytes exactly;
//! out-of-gamut pixels keep their hue and land on the sRGB boundary.
//!
//! `f32` throughout, one `OKLab` cusp solve and one Halley refinement per
//! out-of-gamut pixel — the same fixed-cost structure the shader runs.
//! `oracle/src/gamut.rs` is the `f64` reference of the same algorithm, and
//! `oracle/src/tone.rs` the `f64` reference of the tone map.

/// Linear Display P3 → linear sRGB — the Bradford-adapted D65 matrix of
/// `present.wgsl`, in `f32`.
const P3_TO_LINEAR_SRGB: [[f32; 3]; 3] = [
    [1.224_940_2, -0.224_940_2, 0.0],
    [-0.042_056_95, 1.042_056_9, 0.0],
    [-0.019_637_55, -0.078_636_05, 1.098_273_6],
];

fn mat3(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0].mul_add(v[0], m[0][1].mul_add(v[1], m[0][2] * v[2])),
        m[1][0].mul_add(v[0], m[1][1].mul_add(v[1], m[1][2] * v[2])),
        m[2][0].mul_add(v[0], m[2][1].mul_add(v[1], m[2][2] * v[2])),
    ]
}

/// The sRGB transfer on an in-gamut channel.
fn srgb_encode(x: f32) -> f32 {
    if x <= 0.003_130_8 {
        x * 12.92
    } else {
        1.055f32.mul_add(x.powf(1.0 / 2.4), -0.055)
    }
}

/// Signed cube root — the `OKLab` LMS nonlinearity for possibly-negative
/// components an out-of-gamut colour produces.
fn cbrt(x: f32) -> f32 {
    x.cbrt()
}

fn srgb_to_oklab(rgb: [f32; 3]) -> [f32; 3] {
    let l_ = cbrt(0.412_221_47f32.mul_add(
        rgb[0],
        0.536_332_55f32.mul_add(rgb[1], 0.051_445_993 * rgb[2]),
    ));
    let m_ = cbrt(0.211_903_5f32.mul_add(
        rgb[0],
        0.680_699_5_f32.mul_add(rgb[1], 0.107_396_96 * rgb[2]),
    ));
    let s_ = cbrt(0.088_302_46f32.mul_add(
        rgb[0],
        0.281_718_85f32.mul_add(rgb[1], 0.629_978_7 * rgb[2]),
    ));
    [
        0.210_454_26f32.mul_add(l_, 0.793_617_8f32.mul_add(m_, -0.004_072_047 * s_)),
        1.977_998_5f32.mul_add(l_, (-2.428_592_2f32).mul_add(m_, 0.450_593_7 * s_)),
        0.025_904_037f32.mul_add(l_, 0.782_771_77f32.mul_add(m_, -0.808_675_77 * s_)),
    ]
}

fn oklab_to_srgb(lab: [f32; 3]) -> [f32; 3] {
    let l_ = 0.396_337_78f32.mul_add(lab[1], 0.215_803_76f32.mul_add(lab[2], lab[0]));
    let m_ = (-0.105_561_35f32).mul_add(lab[1], (-0.063_854_17f32).mul_add(lab[2], lab[0]));
    let s_ = (-0.089_484_18f32).mul_add(lab[1], (-1.291_485_5f32).mul_add(lab[2], lab[0]));
    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;
    [
        4.076_741_7f32.mul_add(l, (-3.307_711_6f32).mul_add(m, 0.230_969_94 * s)),
        (-1.268_438f32).mul_add(l, 2.609_757_4f32.mul_add(m, -0.341_319_38 * s)),
        (-0.004_196_086_3f32).mul_add(l, (-0.703_418_6f32).mul_add(m, 1.707_614_7 * s)),
    ]
}

fn in_gamut(rgb: [f32; 3]) -> bool {
    rgb.iter().all(|&c| (0.0..=1.0).contains(&c))
}

/// The cubic and its two derivatives of an sRGB channel along the `L=1`
/// saturation ray, evaluated at `s`.
fn sat_eval(s: f32, w: [f32; 3], kl: [f32; 3]) -> [f32; 3] {
    let l_ = s.mul_add(kl[0], 1.0);
    let m_ = s.mul_add(kl[1], 1.0);
    let s_ = s.mul_add(kl[2], 1.0);
    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let sc = s_ * s_ * s_;
    let l_ds = 3.0 * kl[0] * l_ * l_;
    let m_ds = 3.0 * kl[1] * m_ * m_;
    let s_ds = 3.0 * kl[2] * s_ * s_;
    let l_ds2 = 6.0 * kl[0] * kl[0] * l_;
    let m_ds2 = 6.0 * kl[1] * kl[1] * m_;
    let s_ds2 = 6.0 * kl[2] * kl[2] * s_;
    [
        w[2].mul_add(sc, w[0].mul_add(l, w[1] * m)),
        w[2].mul_add(s_ds, w[0].mul_add(l_ds, w[1] * m_ds)),
        w[2].mul_add(s_ds2, w[0].mul_add(l_ds2, w[1] * m_ds2)),
    ]
}

/// The maximum saturation `S = C/L` for the normalized hue direction
/// `(a, b)` — `ok_color.h`'s `compute_max_saturation` structure in `f32`.
/// All three channel surfaces are solved and the smallest converged root
/// wins: the reference's fitted hue partition is a ~1e-6-sensitive coin
/// flip at the sRGB vertex hues across `f32`/`f64`.
fn max_saturation(a: f32, b: f32) -> f32 {
    // Per channel: five polynomial coefficients, then the LMS->sRGB
    // channel weights.
    const SETS: [[f32; 5]; 3] = [
        [
            1.190_862_8,
            1.765_767_3,
            0.596_626_4,
            0.755_152,
            0.567_712_4,
        ],
        [
            0.739_565_13,
            -0.459_544_03,
            0.082_854_27,
            0.125_410_69,
            0.145_032_05,
        ],
        [
            1.357_336_5,
            -0.009_157_99,
            -1.151_302_1,
            -0.505_596,
            0.006_921_67,
        ],
    ];
    const W: [[f32; 3]; 3] = [
        [4.076_741_7, -3.307_711_6, 0.230_969_94],
        [-1.268_438, 2.609_757_4, -0.341_319_38],
        [-0.004_196_086_3, -0.703_418_6, 1.707_614_7],
    ];
    let kl = [
        0.215_803_76f32.mul_add(b, 0.396_337_78 * a),
        (-0.063_854_17f32).mul_add(b, -0.105_561_35 * a),
        (-1.291_485_5f32).mul_add(b, -0.089_484_18 * a),
    ];
    let mut s = f32::INFINITY;
    let mut fallback = f32::INFINITY;
    for (k, w) in SETS.iter().zip(W.iter()) {
        let est = k[4].mul_add(
            a * b,
            k[3].mul_add(a * a, k[2].mul_add(b, k[1].mul_add(a, k[0]))),
        );
        if est > 0.0 {
            fallback = fallback.min(est);
        }
        let [f, f1, f2] = sat_eval(est, *w, kl);
        let den = f1.mul_add(f1, -0.5 * f * f2);
        if den == 0.0 {
            continue;
        }
        let root = est - f * f1 / den;
        // Keep the root only if the step converged onto the surface — a
        // diverged step leaves a large residual at its landing point.
        if root.is_finite() && root > 0.0 && sat_eval(root, *w, kl)[0].abs() < 0.05 {
            s = s.min(root);
        }
    }
    if s.is_finite() { s } else { fallback.max(0.0) }
}

/// The sRGB cusp `(L, C)` of the hue slice — `ok_color.h`'s `find_cusp`.
fn cusp(a: f32, b: f32) -> (f32, f32) {
    let s = max_saturation(a, b);
    let rgb = oklab_to_srgb([1.0, s * a, s * b]);
    let l_cusp = cbrt(1.0 / rgb[0].max(rgb[1]).max(rgb[2]));
    (l_cusp, l_cusp * s)
}

/// Intersects the segment `(l0, 0)` → `(l1, c1)` with the gamut boundary
/// — `ok_color.h`'s `find_gamut_intersection`.
#[allow(clippy::similar_names, clippy::many_single_char_names)] // mirror ok_color.h
fn gamut_intersection(a: f32, b: f32, l1: f32, c1: f32, l0: f32, cusp: (f32, f32)) -> f32 {
    let (cusp_l, cusp_c) = cusp;
    let mut t;
    if (l1 - l0).mul_add(cusp_c, -(cusp_l - l0) * c1) <= 0.0 {
        t = cusp_c * l0 / c1.mul_add(cusp_l, cusp_c * (l0 - l1));
    } else {
        t = cusp_c * (l0 - 1.0) / c1.mul_add(cusp_l - 1.0, cusp_c * (l0 - l1));
        let dl = l1 - l0;
        let dc = c1;
        let k_l = 0.215_803_76f32.mul_add(b, 0.396_337_78 * a);
        let k_m = (-0.063_854_17f32).mul_add(b, -0.105_561_35 * a);
        let k_s = (-1.291_485_5f32).mul_add(b, -0.089_484_18 * a);
        let l_dt = dc.mul_add(k_l, dl);
        let m_dt = dc.mul_add(k_m, dl);
        let s_dt = dc.mul_add(k_s, dl);
        let l_at = l0.mul_add(1.0 - t, t * l1);
        let c_at = t * c1;
        let l_ = c_at.mul_add(k_l, l_at);
        let m_ = c_at.mul_add(k_m, l_at);
        let s_ = c_at.mul_add(k_s, l_at);
        let l = l_ * l_ * l_;
        let m = m_ * m_ * m_;
        let s = s_ * s_ * s_;
        let ldt = 3.0 * l_dt * l_ * l_;
        let mdt = 3.0 * m_dt * m_ * m_;
        let sdt = 3.0 * s_dt * s_ * s_;
        let ldt2 = 6.0 * l_dt * l_dt * l_;
        let mdt2 = 6.0 * m_dt * m_dt * m_;
        let sdt2 = 6.0 * s_dt * s_dt * s_;
        let r = 4.076_741_7f32.mul_add(l, (-3.307_711_6f32).mul_add(m, 0.230_969_94 * s)) - 1.0;
        let r1 = 4.076_741_7f32.mul_add(ldt, (-3.307_711_6f32).mul_add(mdt, 0.230_969_94 * sdt));
        let r2 = 4.076_741_7f32.mul_add(ldt2, (-3.307_711_6f32).mul_add(mdt2, 0.230_969_94 * sdt2));
        let g = (-1.268_438f32).mul_add(l, 2.609_757_4f32.mul_add(m, -0.341_319_38 * s)) - 1.0;
        let g1 = (-1.268_438f32).mul_add(ldt, 2.609_757_4f32.mul_add(mdt, -0.341_319_38 * sdt));
        let g2 = (-1.268_438f32).mul_add(ldt2, 2.609_757_4f32.mul_add(mdt2, -0.341_319_38 * sdt2));
        let bc =
            (-0.004_196_086_3f32).mul_add(l, (-0.703_418_6f32).mul_add(m, 1.707_614_7 * s)) - 1.0;
        let b1 =
            (-0.004_196_086_3f32).mul_add(ldt, (-0.703_418_6f32).mul_add(mdt, 1.707_614_7 * sdt));
        let b2 = (-0.004_196_086_3f32)
            .mul_add(ldt2, (-0.703_418_6f32).mul_add(mdt2, 1.707_614_7 * sdt2));
        let u_r = r1 / r1.mul_add(r1, -0.5 * r * r2);
        let u_g = g1 / g1.mul_add(g1, -0.5 * g * g2);
        let u_b = b1 / b1.mul_add(b1, -0.5 * bc * b2);
        let t_r = if u_r >= 0.0 { -r * u_r } else { f32::MAX };
        let t_g = if u_g >= 0.0 { -g * u_g } else { f32::MAX };
        let t_b = if u_b >= 0.0 { -bc * u_b } else { f32::MAX };
        t += t_r.min(t_g).min(t_b);
    }
    t
}

/// The analytic `OKLab` clip — `ok_color.h`'s
/// `gamut_clip_adaptive_L0_L_cusp` (`alpha = 0.05`): the projection
/// anchors at the colour's own lightness near the gamut and blends to
/// the cusp's as the colour recedes. Hue is preserved exactly; a final
/// clamp absorbs the triangle approximation's residual.
#[allow(clippy::many_single_char_names)] // l/a/b/c mirror OKLab notation
fn gamut_map(rgb: [f32; 3]) -> [f32; 3] {
    if in_gamut(rgb) {
        return rgb;
    }
    let [l, a, b] = srgb_to_oklab(rgb);
    // Local-MINDE (ΔE_OK JND = 0.02): when the plain channel-clip is
    // already within a JND of the colour, keep the clip's bytes — the
    // pre-#96 output. A colour a few ULPs out of gamut (an in-sRGB value
    // round-tripped through the P3 matrices) then never enters the
    // projection where the cusp fit is weakest.
    let clipped = rgb.map(|c| c.clamp(0.0, 1.0));
    if delta_e_ok(srgb_to_oklab(clipped), [l, a, b]) < 0.02 {
        return clipped;
    }
    let chroma = a.hypot(b);
    if chroma <= 0.000_01 {
        return oklab_to_srgb([l.clamp(0.0, 1.0), 0.0, 0.0]).map(|c| c.clamp(0.0, 1.0));
    }
    let c = chroma.max(0.000_01);
    let a_ = a / c;
    let b_ = b / c;
    let cusp = cusp(a_, b_);
    let ld = l - cusp.0;
    let k = 2.0 * if ld > 0.0 { 1.0 - cusp.0 } else { cusp.0 };
    let e1 = 0.05f32.mul_add(c / k, 0.5f32.mul_add(k, ld.abs()));
    let root = e1.mul_add(e1, -2.0 * k * ld.abs()).sqrt();
    let l0 = (0.5 * ld.signum()).mul_add(e1 - root, cusp.0);
    let t = gamut_intersection(a_, b_, l, c, l0, cusp);
    let l_out = l0.mul_add(1.0 - t, t * l);
    let c_out = t * c;
    oklab_to_srgb([l_out, c_out * a_, c_out * b_]).map(|c| c.clamp(0.0, 1.0))
}

/// Euclidean distance in `OKLab` — `ΔE_OK`.
fn delta_e_ok(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d0 = a[0] - b[0];
    let d1 = a[1] - b[1];
    let d2 = a[2] - b[2];
    d0.mul_add(d0, d1.mul_add(d1, d2 * d2)).sqrt()
}

/// The presentation tone-map shoulder (`f32` twin of `oracle::tone`) —
/// identity on `[0, 1]`, then highlights compress smoothly towards the
/// headroom `h`.
fn tone(x: f32, h: f32) -> f32 {
    if x <= 1.0 {
        return x;
    }
    let d = (h - 1.0).max(0.0);
    let t = x - 1.0;
    d.mul_add(1.0 - d / t.mul_add(1.0, d), 1.0)
}

/// One pixel's tone map (#97): scale every channel by the shoulder's
/// value at the maximum channel — a per-pixel scalar, so hue and
/// saturation are preserved. `max ≤ 1` returns the input bit-for-bit.
fn tone_map(headroom: f32, rgb: [f32; 3]) -> [f32; 3] {
    let m = rgb[0].max(rgb[1]).max(rgb[2]);
    if m.partial_cmp(&1.0) != Some(std::cmp::Ordering::Greater) {
        return rgb;
    }
    let s = tone(m, headroom) / m;
    [rgb[0] * s, rgb[1] * s, rgb[2] * s]
}

/// Presents premultiplied working-space pixels to encoded premultiplied
/// sRGB bytes.
///
/// The `present.wgsl` `srgb-shader` path under `OutputAlpha::Premultiplied`,
/// ending in the unorm-8 store an sRGB destination produces. Alpha is
/// stored linearly, as `Rgba8*` formats do. `headroom` is the display's
/// headroom; an sRGB destination's representable ceiling caps the
/// tone-map target at `1`.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "gamut-mapped channels are in [0,1]; the unorm-8 store rounds"
)]
pub fn present_srgb8(headroom: f32, pixels: &[[f32; 4]]) -> Vec<u8> {
    let target = headroom.min(1.0);
    let mut out = Vec::with_capacity(pixels.len() * 4);
    for &[r, g, b, a] in pixels {
        let straight = if a > 0.0 {
            tone_map(target, [r / a, g / a, b / a])
        } else {
            [0.0; 3]
        };
        let mapped = gamut_map(mat3(&P3_TO_LINEAR_SRGB, straight));
        for c in mapped {
            out.push((srgb_encode(c) * a * 255.0).round() as u8);
        }
        out.push((a.clamp(0.0, 1.0) * 255.0).round() as u8);
    }
    out
}

/// Presents premultiplied working-space pixels to an extended linear
/// Display P3 host buffer (#97).
///
/// The CPU twin of `present.wgsl`'s `OutputColor::LinearDisplayP3` path:
/// unpremultiply, tone-map to the display `headroom`, re-premultiply.
#[must_use]
pub fn present_linear_p3(headroom: f32, pixels: &[[f32; 4]]) -> Vec<[f32; 4]> {
    pixels
        .iter()
        .map(|&[r, g, b, a]| {
            // The tone map scales straight rgb by a scalar, so premultiplied
            // channels scale by the same factor directly — no divide and
            // re-multiply round trip, and a pixel whose straight channels
            // are all ≤ 1 returns bit-for-bit (SDR identity).
            if a.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
                return [0.0, 0.0, 0.0, a];
            }
            let m = r.max(g).max(b) / a;
            if m.partial_cmp(&1.0) != Some(std::cmp::Ordering::Greater) {
                return [r, g, b, a];
            }
            let s = tone(m, headroom) / m;
            [r * s, g * s, b * s, a]
        })
        .collect()
}
