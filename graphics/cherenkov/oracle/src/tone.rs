//! Tone mapping to the display's headroom — the `f64` reference for the
//! presentation pass's extended-range handling (#97).
//!
//! The headroom `h ≥ 1` is the destination's peak luminance as a multiple
//! of SDR white. The map leaves `[0, 1]` — the SDR range — untouched and
//! compresses values above `1.0` smoothly into `[1, h]`. An SDR target is
//! `h = 1`: the highlight range collapses onto `1.0`, approached as the
//! `h → 1` limit of the shoulder rather than a hard clip on the input.
//!
//! The map scales all channels by the shoulder's value at the maximum
//! channel — a per-pixel scalar, so hue and saturation are preserved
//! exactly and a saturated highlight never desaturates to white. Applying
//! it to the maximum channel also bounds the output: `max(out) = f(max(in))
//! ≤ h`, the contract the headroom expresses.
//!
//! The shoulder is the extended-Reinhard/EDR curve `1 + (h−1)·t/(t + h−1)`
//! on the excess `t = x − 1`: `C1` at the knee, asymptotic to `h`, so
//! highlights keep their ordering at every input extent. It was measured
//! against a BT.2390-adapted EETF (`cherenkov-bench tone-sweep` keeps it
//! as the measurement reference): the EETF plateaus at `3h−2`, so at
//! headroom 2 every input above `4×` maps to the same value — a hard clip
//! across most of an `0..8×` ramp — while this shoulder stays strictly
//! increasing for all finite inputs. Both were `C1` at the knee and equal
//! in present-pass cost on lavapipe; the same hue-preserving `max`
//! scaling applies to either, so the plateau decided it.

/// The presentation tone-map shoulder (#97).
///
/// `1 + Δ·t/(t+Δ)` on the excess `t = x − 1`, asymptotic to `h` — the
/// curve selected by the `tone-sweep` measurement and the
/// headroom-1/2/4 visual review. Written as `1 + Δ·(1 − Δ/(t+Δ))` so
/// `x = +∞` returns `h` rather than `NaN`.
///
/// `h` is the display headroom (`≥ 1`); smaller values are treated as
/// `1`.
#[must_use]
pub fn tone(x: f64, headroom: f64) -> f64 {
    if x <= 1.0 {
        return x;
    }
    let d = (headroom - 1.0).max(0.0);
    let t = x - 1.0;
    d.mul_add(1.0 - d / t.mul_add(1.0, d), 1.0)
}

/// One pixel's tone map: scale every channel by the shoulder's value at
/// the maximum channel. A `max ≤ 1` pixel — all SDR content — returns
/// bit-for-bit.
///
/// `rgb` is a straight (unpremultiplied) colour in the working space —
/// the map runs before the destination's gamut conversion, so an
/// SDR-intensity P3 colour never takes the shoulder; `headroom` is the
/// destination's peak in units of SDR white.
#[must_use]
pub fn tone_map(headroom: f64, rgb: [f64; 3]) -> [f64; 3] {
    let m = rgb[0].max(rgb[1]).max(rgb[2]);
    if m.partial_cmp(&1.0) != Some(std::cmp::Ordering::Greater) {
        return rgb;
    }
    let s = tone(m, headroom) / m;
    [rgb[0] * s, rgb[1] * s, rgb[2] * s]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdr_range_is_identity() {
        for &x in &[0.0, 0.25, 0.5, 0.999_999, 1.0] {
            assert_eq!(tone(x, 4.0).to_bits(), x.to_bits(), "x={x}");
        }
    }

    #[test]
    fn shoulder_starts_at_one_with_slope_one() {
        for &h in &[1.5, 2.0, 4.0, 16.0] {
            assert_eq!(tone(1.0, h).to_bits(), 1.0f64.to_bits());
            // C1 at the knee: the right derivative matches the SDR slope.
            let eps = 1e-6;
            let slope = (tone(1.0 + eps, h) - 1.0) / eps;
            assert!((slope - 1.0).abs() < 1e-3, "h={h} slope={slope}");
        }
        // H = 1 is the degenerate ceiling: everything above 1 lands on 1.
        assert_eq!(tone(1.0 + 1e-9, 1.0).to_bits(), 1.0f64.to_bits());
    }

    #[test]
    fn bounded_by_headroom_and_monotone() {
        for &h in &[1.0, 1.5, 2.0, 4.0, 16.0] {
            let mut prev = f64::NEG_INFINITY;
            for i in 0..4096 {
                let x = f64::from(i).mul_add(0.01, 1.0);
                let y = tone(x, h);
                assert!(y >= prev, "h={h} x={x}: {y} < {prev}");
                assert!(y <= h, "h={h} x={x}: {y} > {h}");
                prev = y;
            }
        }
    }

    #[test]
    fn map_scales_channels_preserving_ratio() {
        let rgb = [4.0, 1.0, 0.25];
        let out = tone_map(2.0, rgb);
        // Max channel lands on the shoulder's value; ratios preserved.
        let m = 4.0_f64;
        assert!((out[0] - tone(m, 2.0)).abs() < 1e-15);
        assert!((out[1] / out[0] - 0.25).abs() < 1e-15);
    }

    #[test]
    fn in_sdr_pixel_passes_through() {
        let rgb = [0.25, 0.5, 0.75];
        assert_eq!(tone_map(4.0, rgb).map(f64::to_bits), rgb.map(f64::to_bits));
        let neg = [-0.2, 0.5, 0.1];
        assert_eq!(tone_map(4.0, neg).map(f64::to_bits), neg.map(f64::to_bits));
    }

    #[test]
    fn sdr_target_compresses_to_one() {
        assert_eq!(tone(4.0, 1.0).to_bits(), 1.0f64.to_bits());
        assert_eq!(tone(100.0, 1.0).to_bits(), 1.0f64.to_bits());
    }

    #[test]
    fn infinite_input_lands_at_headroom() {
        assert_eq!(tone(f64::INFINITY, 2.0).to_bits(), 2.0f64.to_bits());
    }
}
