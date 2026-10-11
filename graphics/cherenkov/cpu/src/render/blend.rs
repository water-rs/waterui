//! W3C Compositing and Blending Level 1 formulas, ported from the
//! oracle's `blend.rs` into `f32`.
//!
//! Blending is done on un-premultiplied channels per the spec:
//! `Cr = (1 - αb)·Cs + αb·B(Cb, Cs)` where `B` is the blend function, then
//! the blended colour `Cr` is composited source-over with source alpha
//! `αs` — in premultiplied terms `Co = αs·Cr + (1 - αs)·Cb'`.

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

fn lum(c: [f32; 3]) -> f32 {
    0.11f32.mul_add(c[2], 0.59f32.mul_add(c[1], 0.3 * c[0]))
}

fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

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
    // Un-premultiply (clamped to [0,1]; components are finite by construction).
    let ub = if ab > 0.0 {
        [cb[0] / ab, cb[1] / ab, cb[2] / ab]
    } else {
        [0.0; 3]
    };
    let us = [cs[0] / as_, cs[1] / as_, cs[2] / as_];
    let b: [f32; 3] = match mode {
        BlendMode::Hue => set_lum(set_sat(us, sat(ub)), lum(ub)),
        BlendMode::Saturation => set_lum(set_sat(ub, sat(us)), lum(ub)),
        BlendMode::Color => set_lum(us, lum(ub)),
        BlendMode::Luminosity => set_lum(ub, lum(us)),
        _ => [
            blend_channel(mode, ub[0], us[0]),
            blend_channel(mode, ub[1], us[1]),
            blend_channel(mode, ub[2], us[2]),
        ],
    };
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
}
