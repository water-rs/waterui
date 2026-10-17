//! W3C Compositing and Blending Level 1 formulas, ported from the
//! oracle's `blend.rs` into `f32`.
//!
//! Blending is done on un-premultiplied channels per the spec:
//! `Cr = (1 - αb)·Cs + αb·B(Cb, Cs)` where `B` is the blend function, then
//! the blended colour `Cr` is composited source-over with source alpha
//! `αs` — in premultiplied terms `Co = αs·Cr + (1 - αs)·Cb'`.
//!
//! Extension over the W3C text (#2908): the four non-separable modes
//! (Hue, Saturation, Color, Luminosity) evaluate `B` on a shared affine
//! domain covering both straight colours instead of the fixed SDR
//! `[0,1]`, so extended-range and negative channels blend through the
//! same formulas rather than clamping. See `blend`'s domain comment.

use cherenkov::BlendMode;

/// Operators where a transparent source changes the destination
/// (`Clear`, `Src`, `SrcIn`, `SrcOut`, `DestIn`, `DestAtop`). Their
/// composite is bounded by the effective clip instead of the tight
/// content bounds.
pub const fn is_destructive(mode: BlendMode) -> bool {
    matches!(
        mode,
        BlendMode::Clear
            | BlendMode::Src
            | BlendMode::SrcIn
            | BlendMode::SrcOut
            | BlendMode::DestIn
            | BlendMode::DestAtop
    )
}

#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn lum(c: [f32; 3]) -> f32 {
    0.11f32.mul_add(c[2], 0.59f32.mul_add(c[1], 0.3 * c[0]))
}

fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn clip_color(c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut c = c;
    if n < 0.0 {
        for ch in &mut c {
            *ch = l + (*ch - l) * l / (l - n);
        }
    }
    if x > 1.0 {
        for ch in &mut c {
            *ch = l + (*ch - l) * (1.0 - l) / (x - l);
        }
    }
    c
}

#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color([c[0] + d, c[1] + d, c[2] + d])
}

#[expect(clippy::similar_names, reason = "names mirror the W3C spec")]
fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let (imin, imax) = {
        let mut mn = 0;
        let mut mx = 0;
        for i in 1..3 {
            if c[i] < c[mn] {
                mn = i;
            }
            if c[i] > c[mx] {
                mx = i;
            }
        }
        (mn, mx)
    };
    let imid = 3 - imin - imax;
    let mut out = [0.0; 3];
    if c[imax] > c[imin] {
        out[imid] = (c[imid] - c[imin]) * s / (c[imax] - c[imin]);
        out[imax] = s;
    }
    out
}

/// The blend function `B(Cb, Cs)` for one channel pair, separable modes.
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
fn blend_channel(mode: BlendMode, cb: f32, cs: f32) -> f32 {
    match mode {
        BlendMode::Multiply => cb * cs,
        BlendMode::Screen => cb.mul_add(-cs, cb + cs),
        BlendMode::Overlay => {
            if cb <= 0.5 {
                2.0 * cb * cs
            } else {
                (2.0 * (1.0 - cb)).mul_add(-(1.0 - cs), 1.0)
            }
        }
        BlendMode::Darken => cb.min(cs),
        BlendMode::Lighten => cb.max(cs),
        BlendMode::ColorDodge => {
            if cs >= 1.0 {
                1.0
            } else {
                (cb / (1.0 - cs)).min(1.0)
            }
        }
        BlendMode::ColorBurn => {
            if cs <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - cb) / cs).min(1.0)
            }
        }
        BlendMode::HardLight => {
            if cs <= 0.5 {
                2.0 * cb * cs
            } else {
                (2.0 * (1.0 - cb)).mul_add(-(1.0 - cs), 1.0)
            }
        }
        BlendMode::SoftLight => {
            if cs <= 0.5 {
                (2.0f32.mul_add(-cs, 1.0) * cb).mul_add(-(1.0 - cb), cb)
            } else {
                let d = if cb <= 0.25 {
                    16.0f32.mul_add(cb, -12.0).mul_add(cb, 4.0) * cb
                } else {
                    cb.sqrt()
                };
                2.0f32.mul_add(cs, -1.0).mul_add(d - cb, cb)
            }
        }
        BlendMode::Difference => (cb - cs).abs(),
        BlendMode::Exclusion => (2.0 * cb).mul_add(-cs, cb + cs),
        // `Normal` and the non-separable modes (evaluated per-pixel in
        // `blend`, not here) pass the source channel through.
        _ => cs,
    }
}

/// Blend premultiplied source `cs` onto premultiplied backdrop `cb`,
/// returning the composited premultiplied `co` directly.
///
/// `Cs' = (1-αb)·Cs + αb·B(Cb, Cs)` and `co = αs·Cs' + αb·Cb·(1-αs)` —
/// so a fully transparent source leaves the backdrop unchanged, and
/// [`BlendMode::Normal`] reduces to source-over.
#[must_use]
#[expect(
    clippy::inline_always,
    reason = "compiled under the dispatched CPU level so mul_add is one instruction"
)]
#[inline(always)]
pub fn blend(mode: BlendMode, cb: [f32; 4], cs: [f32; 4]) -> [f32; 4] {
    let (ab, as_) = (cb[3], cs[3]);
    // Porter-Duff compositing operators (COLRv1 `PaintComposite`): no
    // colour blending, `co = αs·Fa·Cs + αb·Fb·Cb` in premultiplied form.
    let porter_duff = |fa: f32, fb: f32| -> [f32; 4] {
        [
            fa.mul_add(cs[0], fb * cb[0]),
            fa.mul_add(cs[1], fb * cb[1]),
            fa.mul_add(cs[2], fb * cb[2]),
            fa.mul_add(as_, fb * ab),
        ]
    };
    match mode {
        BlendMode::Clear => return [0.0; 4],
        BlendMode::Src => return cs,
        BlendMode::Dst => return cb,
        BlendMode::DestOver => return porter_duff(1.0 - ab, 1.0),
        BlendMode::SrcIn => return porter_duff(ab, 0.0),
        BlendMode::DestIn => return porter_duff(0.0, as_),
        BlendMode::SrcOut => return porter_duff(1.0 - ab, 0.0),
        BlendMode::DestOut => return porter_duff(0.0, 1.0 - as_),
        BlendMode::SrcAtop => return porter_duff(ab, 1.0 - as_),
        BlendMode::DestAtop => return porter_duff(1.0 - ab, as_),
        BlendMode::Xor => return porter_duff(1.0 - ab, 1.0 - as_),
        BlendMode::PlusLighter => {
            // Additive light: colour sums unclamped; coverage saturates.
            return [
                cs[0] + cb[0],
                cs[1] + cb[1],
                cs[2] + cb[2],
                (as_ + ab).min(1.0),
            ];
        }
        _ => {}
    }
    if as_ == 0.0 {
        return cb;
    }
    // Un-premultiply; components are finite by construction and keep
    // their extended range (HDR and negative channels pass through).
    let ub = if ab > 0.0 {
        [cb[0] / ab, cb[1] / ab, cb[2] / ab]
    } else {
        [0.0; 3]
    };
    let us = [cs[0] / as_, cs[1] / as_, cs[2] / as_];
    // #2908: the non-separable modes share one affine domain covering
    // both straight colours — `lo` = min(0, all six channels), `hi` =
    // max(1, all six), so the domain always contains [0,1]. When the
    // domain is extended, both colours normalize through
    // `scale = max(-lo, hi)` as `(c/scale - lo/scale) / (hi/scale -
    // lo/scale)` — the same affine map as `(c - lo)/(hi - lo)` with every
    // intermediate bounded in magnitude by 2, so even ±f32::MAX channels
    // stay finite
    // where the direct width `hi - lo` would overflow. `nb`/`ns` are the
    // colours the SDR formulas below see; a pure-SDR pair hits the
    // identity domain and keeps the exact original arithmetic. Since
    // `lum` and `sat` are affine-covariant the modes keep their
    // semantics; the blend result maps back through `lo·(1-B) + hi·B`
    // (the convex combination `lo + (hi-lo)·B` written with
    // opposite-sign terms so no intermediate can overflow), and the
    // source-over tail still uses the unnormalized `us`.
    let (mut nb, mut ns, mut lo, mut hi) = (ub, us, 0.0f32, 1.0f32);
    let mut normalized = false;
    if matches!(
        mode,
        BlendMode::Hue | BlendMode::Saturation | BlendMode::Color | BlendMode::Luminosity
    ) {
        lo = ub.iter().chain(us.iter()).fold(0.0f32, |m, &c| m.min(c));
        hi = ub.iter().chain(us.iter()).fold(1.0f32, |m, &c| m.max(c));
        if lo < 0.0 || hi > 1.0 {
            let scale = (-lo).max(hi);
            let low_scaled = lo / scale;
            let width_scaled = hi / scale - low_scaled;
            nb = [
                (ub[0] / scale - low_scaled) / width_scaled,
                (ub[1] / scale - low_scaled) / width_scaled,
                (ub[2] / scale - low_scaled) / width_scaled,
            ];
            ns = [
                (us[0] / scale - low_scaled) / width_scaled,
                (us[1] / scale - low_scaled) / width_scaled,
                (us[2] / scale - low_scaled) / width_scaled,
            ];
            normalized = true;
        }
    }
    let mut b: [f32; 3] = match mode {
        BlendMode::Hue => set_lum(set_sat(ns, sat(nb)), lum(nb)),
        BlendMode::Saturation => set_lum(set_sat(nb, sat(ns)), lum(nb)),
        BlendMode::Color => set_lum(ns, lum(nb)),
        BlendMode::Luminosity => set_lum(nb, lum(ns)),
        _ => [
            blend_channel(mode, ub[0], us[0]),
            blend_channel(mode, ub[1], us[1]),
            blend_channel(mode, ub[2], us[2]),
        ],
    };
    if normalized {
        b = [
            hi.mul_add(b[0], lo * (1.0 - b[0])),
            hi.mul_add(b[1], lo * (1.0 - b[1])),
            hi.mul_add(b[2], lo * (1.0 - b[2])),
        ];
    }
    // Cr = (1-αb)·Cs + αb·B(Cb,Cs); premultiplied output: αs·Cr + (1-αs)·Cb.
    let mut out = [0.0; 4];
    for i in 0..3 {
        let cr = ab.mul_add(b[i], (1.0 - ab) * us[i]);
        out[i] = (1.0 - as_).mul_add(cb[i], as_ * cr);
    }
    out[3] = ab.mul_add(1.0 - as_, as_);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plus_lighter_saturates_alpha_but_not_colour() {
        // Two opaque sources: coverage saturates at 1, light adds
        // unclamped — extended-range colour survives (#126).
        let out = blend(
            BlendMode::PlusLighter,
            [0.5, 0.0, 0.0, 1.0],
            [1.0, 0.0, 0.5, 1.0],
        );
        assert_eq!(
            out.map(f32::to_bits),
            [1.5, 0.0, 0.5, 1.0].map(f32::to_bits)
        );

        let out = blend(
            BlendMode::PlusLighter,
            [0.25, 0.25, 0.25, 0.5],
            [0.5, 0.0, 0.0, 1.0],
        );
        assert_eq!(
            out.map(f32::to_bits),
            [0.75, 0.25, 0.25, 1.0].map(f32::to_bits)
        );
    }

    const NONSEPARABLE: [BlendMode; 4] = [
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
    ];

    /// #2908: the non-separable modes share an affine domain over both
    /// straight colours, so an identical extended-range grey survives the
    /// blend untouched instead of being dragged into [0,1] by the clip
    /// inside `set_lum`.
    #[test]
    fn nonseparable_identical_extended_grey_is_unchanged() {
        for mode in NONSEPARABLE {
            for g in [4.0_f32, -2.0, 0.5] {
                let out = blend(mode, [g, g, g, 1.0], [g, g, g, 1.0]);
                assert!(
                    out.iter().all(|c| c.is_finite()),
                    "{mode:?} on grey {g}: {out:?}"
                );
                assert_eq!(
                    out.map(f32::to_bits),
                    [g, g, g, 1.0].map(f32::to_bits),
                    "{mode:?} on identical grey {g}"
                );
            }
        }
    }

    /// Backdrop grey 4, source grey 2, both opaque: Hue, Saturation and
    /// Colour keep the backdrop's luminance (grey 4), Luminosity takes
    /// the source's (grey 2) — in the extended range, not folded to SDR.
    #[test]
    fn nonseparable_hdr_greys_keep_their_luminance_semantics() {
        for (mode, want) in [
            (BlendMode::Hue, 4.0_f32),
            (BlendMode::Saturation, 4.0),
            (BlendMode::Color, 4.0),
            (BlendMode::Luminosity, 2.0),
        ] {
            let out = blend(mode, [4.0, 4.0, 4.0, 1.0], [2.0, 2.0, 2.0, 1.0]);
            assert_eq!(
                out.map(f32::to_bits),
                [want, want, want, 1.0].map(f32::to_bits),
                "{mode:?} grey4 x grey2"
            );
        }
    }

    /// The normalization must not leak into the composite: the source-over
    /// tail uses the unnormalized `us`. Source straight grey 2 at
    /// αs = 0.5 over opaque grey 4 — H/S/C keep `Cr = 4`, so the output is
    /// `0.5·4 + 0.5·4 = 4`; Luminosity's `Cr = 2` gives `0.5·2 + 0.5·4
    /// = 3`. Reversed (opaque grey-4 source over half-alpha grey-2
    /// backdrop) `Cr` mixes `αb·B + (1-αb)·Cs` = 3 or 4.
    #[test]
    fn nonseparable_hdr_greys_composite_source_over() {
        for (mode, want) in [
            (BlendMode::Hue, 4.0_f32),
            (BlendMode::Saturation, 4.0),
            (BlendMode::Color, 4.0),
            (BlendMode::Luminosity, 3.0),
        ] {
            let out = blend(mode, [4.0, 4.0, 4.0, 1.0], [1.0, 1.0, 1.0, 0.5]);
            assert_eq!(
                out.map(f32::to_bits),
                [want, want, want, 1.0].map(f32::to_bits),
                "{mode:?} half-alpha source"
            );
        }
        for (mode, want) in [
            (BlendMode::Hue, 3.0_f32),
            (BlendMode::Saturation, 3.0),
            (BlendMode::Color, 3.0),
            (BlendMode::Luminosity, 4.0),
        ] {
            let out = blend(mode, [1.0, 1.0, 1.0, 0.5], [4.0, 4.0, 4.0, 1.0]);
            assert_eq!(
                out.map(f32::to_bits),
                [want, want, want, 1.0].map(f32::to_bits),
                "{mode:?} half-alpha backdrop"
            );
        }
    }

    /// Chromatic wide-range input — negative and HDR channels across both
    /// colours — stays finite through the affine domain and the clip
    /// inside `set_lum`; no NaN escapes the normalization. The scaled
    /// domain keeps the extremes too: opposite-sign `±f32::MAX` endpoints
    /// would overflow the direct width `hi - lo`.
    #[test]
    fn nonseparable_wide_range_stays_finite() {
        for mode in NONSEPARABLE {
            let out = blend(mode, [-2.0, 4.0, 0.25, 1.0], [8.0, -1.0, 2.0, 1.0]);
            assert!(
                out.iter().all(|c| c.is_finite()),
                "{mode:?} opaque wide range: {out:?}"
            );
            let out = blend(mode, [-1.0, -2.0, 1.5, 0.5], [4.0, 3.0, -0.5, 0.75]);
            assert!(
                out.iter().all(|c| c.is_finite()),
                "{mode:?} translucent wide range: {out:?}"
            );
            let out = blend(
                mode,
                [-f32::MAX, f32::MAX, 0.5, 1.0],
                [f32::MAX, -f32::MAX, 2.0, 1.0],
            );
            assert!(
                out.iter().all(|c| c.is_finite()),
                "{mode:?} opposite-sign f32::MAX endpoints: {out:?}"
            );
            for g in [f32::MAX, -f32::MAX] {
                let out = blend(mode, [g, g, g, 1.0], [g, g, g, 1.0]);
                for c in &out[..3] {
                    assert!(c.is_finite(), "{mode:?} grey {g}: {out:?}");
                    assert!(((c - g) / g).abs() <= 2e-6, "{mode:?} grey {g}: {out:?}");
                }
                assert_eq!(out[3].to_bits(), 1.0f32.to_bits(), "{mode:?} grey {g}");
            }
        }
    }

    /// Pure-SDR inputs take the identity domain (`lo = 0`, `hi = 1`) and
    /// keep the original arithmetic: the analytical values for an opaque
    /// blue source over an opaque red backdrop. Hue/Colour are
    /// `SetLum(blue, 0.3)` = [0.19, 0.19, 1.19], then `ClipColor`'s high
    /// clip at x = 1.19, l = 0.3 pulls the low channels to
    /// `0.3 - 0.11·0.7/0.89` = 19/89; Luminosity's clip at n = −0.19,
    /// l = 0.11 puts red at `0.11/0.3` = 11/30.
    #[test]
    fn nonseparable_sdr_results_unchanged() {
        for (mode, want) in [
            (BlendMode::Hue, [19.0 / 89.0, 19.0 / 89.0, 1.0, 1.0]),
            (BlendMode::Saturation, [1.0, 0.0, 0.0, 1.0]),
            (BlendMode::Color, [19.0 / 89.0, 19.0 / 89.0, 1.0, 1.0]),
            (BlendMode::Luminosity, [11.0 / 30.0, 0.0, 0.0, 1.0]),
        ] {
            let out = blend(mode, [1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]);
            for (got, want) in out.iter().zip(want) {
                assert!((*got - want).abs() < 1e-6, "{mode:?}: {out:?} != {want:?}");
            }
        }
    }
}