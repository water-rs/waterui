//! Colour packing for the HWUI target.
//!
//! The working space is linear Display P3 with straight, extended-range
//! components. HWUI composites in the window's encoded colour space, so a
//! colour crosses as an Android `ColorLong` in `EXTENDED_SRGB`: the working
//! value converted to linear sRGB and encoded with the sign-preserving sRGB
//! transfer, which represents every working colour, out-of-gamut and HDR
//! included, without clamping. The window's own space then decides how much
//! of it shows.

use waterui_graphics::draw::color::{ColorSpace as _, LinearDisplayP3, WorkingColor};
use waterui_graphics::draw::{ColorStop, Extend, Interpolation};

/// `ColorSpace.Named.EXTENDED_SRGB`'s id.
pub const EXTENDED_SRGB: u64 = 2;

/// The largest gap, in encoded units, between a gradient's linear-light
/// interpolation and the encoded interpolation HWUI performs: half an
/// 8-bit step.
const GRADIENT_TOLERANCE: f32 = 0.5 / 255.0;
/// Bisection depth per stop interval: at most 64 extra stops per interval.
const GRADIENT_DEPTH: u32 = 6;

/// `x` encoded with the sRGB transfer, mirrored for negative values.
#[must_use]
pub fn srgb_encode(x: f32) -> f32 {
    let magnitude = x.abs();
    let encoded = if magnitude <= 0.003_130_8 {
        magnitude * 12.92
    } else {
        1.055f32.mul_add(magnitude.powf(1.0 / 2.4), -0.055)
    };
    encoded.copysign(x)
}

/// A working colour as extended-sRGB encoded components, straight alpha.
#[must_use]
pub fn encoded(color: WorkingColor) -> [f32; 4] {
    let [r, g, b, a] = color.components;
    let [r, g, b] = LinearDisplayP3::to_linear_srgb([r, g, b]);
    [srgb_encode(r), srgb_encode(g), srgb_encode(b), a]
}

/// A working colour as linear extended-sRGB components, straight alpha:
/// what a mesh's vertex colours carry, so the rasteriser interpolates in
/// linear light.
#[must_use]
pub fn linear_srgb(color: WorkingColor) -> [f32; 4] {
    let [r, g, b, a] = color.components;
    let [r, g, b] = LinearDisplayP3::to_linear_srgb([r, g, b]);
    [r, g, b, a]
}

/// A working colour as premultiplied linear extended-sRGB components: what
/// a mesh patch's corners carry, since the contract interpolates mesh
/// colours premultiplied.
#[must_use]
pub fn premultiplied_linear_srgb(color: WorkingColor) -> [f32; 4] {
    premultiplied_linear(color)
}

/// A working colour as an `EXTENDED_SRGB` `ColorLong`, alpha scaled by
/// `alpha`.
#[must_use]
pub fn color_long(color: WorkingColor, alpha: f32) -> u64 {
    let [r, g, b, a] = encoded(color);
    pack(r, g, b, a * alpha)
}

/// `Color.pack(r, g, b, a, EXTENDED_SRGB)`: three half floats, a 10-bit
/// alpha and the space id.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the alpha is clamped to 0..=1 and scaled to 0..=1023 first, exactly as Color.pack does"
)]
pub fn pack(r: f32, g: f32, b: f32, a: f32) -> u64 {
    let alpha = a.clamp(0.0, 1.0).mul_add(1023.0, 0.5) as u64;
    (u64::from(half(r)) << 48)
        | (u64::from(half(g)) << 32)
        | (u64::from(half(b)) << 16)
        | ((alpha & 0x3ff) << 6)
        | EXTENDED_SRGB
}

/// `value` as IEEE 754 binary16 bits, rounded to nearest even, as
/// `android.util.Half.toHalf` rounds.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "bit-level float conversion: every narrowed value is masked or range-checked first"
)]
pub const fn half(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x007f_ffff;
    if exponent == 0xff {
        return sign | 0x7c00 | if mantissa == 0 { 0 } else { 0x0200 };
    }
    let rebased = exponent - 127 + 15;
    if rebased >= 0x1f {
        return sign | 0x7c00;
    }
    if rebased <= 0 {
        if rebased < -10 {
            return sign;
        }
        let full = mantissa | 0x0080_0000;
        let shift = (14 - rebased) as u32;
        let kept = full >> shift;
        let rest = full & ((1 << shift) - 1);
        let halfway = 1 << (shift - 1);
        let rounded = if rest > halfway || (rest == halfway && kept & 1 == 1) {
            kept + 1
        } else {
            kept
        };
        return sign | rounded as u16;
    }
    let kept = mantissa >> 13;
    let rest = mantissa & 0x1fff;
    let mut out = ((rebased as u32) << 10) | kept;
    if rest > 0x1000 || (rest == 0x1000 && kept & 1 == 1) {
        out += 1;
    }
    sign | out as u16
}

/// A gradient's stops as `ColorLong`s and positions. A `Working`
/// gradient interpolates in linear light; HWUI interpolates the encoded
/// stops, so each interval is bisected until the encoded interpolation
/// stays within [`GRADIENT_TOLERANCE`] of the linear one.
pub fn gradient_stops(
    stops: &[ColorStop],
    interpolation: Interpolation,
    colors: &mut Vec<u64>,
    positions: &mut Vec<f32>,
) {
    colors.clear();
    positions.clear();
    let mut previous: Option<(f32, [f32; 4])> = None;
    for stop in stops {
        let linear = premultiplied_linear(stop.color);
        if let (Some((offset, from)), Interpolation::Working) = (previous, interpolation) {
            subdivide(
                offset,
                from,
                stop.offset,
                linear,
                GRADIENT_DEPTH,
                colors,
                positions,
            );
        }
        colors.push(color_long(stop.color, 1.0));
        positions.push(stop.offset);
        previous = Some((stop.offset, linear));
    }
}

/// The most stops one turn of a sweep gradient may carry once its extend
/// replicates them.
pub const MAX_SWEEP_STOPS: usize = 4096;

/// A transparent `ColorLong`.
const TRANSPARENT: u64 = EXTENDED_SRGB;

/// Re-maps a sweep's stops, `colors` at `positions` over `t` in `0..=1`,
/// onto the turn `SweepGradient` spans, where the angle fraction `u`
/// stands for `t = k·u` (`k`, turn over span, is positive): a span over a
/// turn cuts the stops at `k`, and a shorter one repeats, mirrors, pads or
/// ends them by `extend` up to `k`, with hard edges as duplicate
/// positions. Cut colours mix the encoded neighbours, as HWUI does
/// between stops. Fails with the stop count past [`MAX_SWEEP_STOPS`].
pub fn sweep_turn(
    k: f64,
    extend: Extend,
    colors: &[u64],
    positions: &[f32],
    out_colors: &mut Vec<u64>,
    out_positions: &mut Vec<f32>,
) -> Result<(), usize> {
    out_colors.clear();
    out_positions.clear();
    let (Some(&first), Some(&last)) = (colors.first(), colors.last()) else {
        return Ok(());
    };
    let mut push = |u: f64, color: u64| {
        out_positions.push(narrow(u).clamp(0.0, 1.0));
        out_colors.push(color);
    };
    let stops = || {
        colors
            .iter()
            .copied()
            .zip(positions.iter().map(|&p| f64::from(p)))
    };
    if k <= 1.0 {
        for (color, t) in stops() {
            if t > k {
                push(1.0, sample(colors, positions, k));
                return Ok(());
            }
            push(t / k, color);
        }
        return Ok(());
    }
    match extend {
        Extend::Pad => stops().for_each(|(color, t)| push(t / k, color)),
        Extend::None => {
            stops().for_each(|(color, t)| push(t / k, color));
            push(1.0 / k, last);
            push(1.0 / k, TRANSPARENT);
            push(1.0, TRANSPARENT);
        }
        Extend::Repeat | Extend::Reflect => {
            // Each cycle carries its ends explicitly, so the next cycle
            // starts on a hard edge.
            let count = colors.len() + 2;
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a positive cycle count, saturating as `as` does"
            )]
            let cycles = k.ceil() as usize;
            if cycles > MAX_SWEEP_STOPS / count {
                return Err(cycles.saturating_mul(count));
            }
            let stop = |index: usize| match index {
                0 => (first, 0.0),
                _ if index == count - 1 => (last, 1.0),
                _ => (colors[index - 1], f64::from(positions[index - 1])),
            };
            for cycle in 0..cycles {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "cycle counts are bounded by the stop budget"
                )]
                let base = cycle as f64;
                let mirrored = extend == Extend::Reflect && cycle % 2 == 1;
                for index in 0..count {
                    let (color, t) = if mirrored {
                        let (color, t) = stop(count - 1 - index);
                        (color, 1.0 - t)
                    } else {
                        stop(index)
                    };
                    let at = base + t;
                    if at > k {
                        let into = k - base;
                        let t = if mirrored { 1.0 - into } else { into };
                        push(1.0, sample(colors, positions, t));
                        return Ok(());
                    }
                    push(at / k, color);
                }
            }
        }
    }
    Ok(())
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "the wire carries f32 stop positions"
)]
const fn narrow(value: f64) -> f32 {
    value as f32
}

/// The colour HWUI shows at `t` of a stop list: its ends pad, and between
/// stops the encoded components mix.
fn sample(colors: &[u64], positions: &[f32], t: f64) -> u64 {
    let t = narrow(t);
    let Some((&first, &last)) = colors.first().zip(colors.last()) else {
        return TRANSPARENT;
    };
    if positions.first().is_none_or(|&p| t <= p) {
        return first;
    }
    for (pair, at) in colors.windows(2).zip(positions.windows(2)) {
        if t <= at[1] {
            let span = at[1] - at[0];
            let fraction = if span > 0.0 { (t - at[0]) / span } else { 0.0 };
            return mix(pair[0], pair[1], fraction);
        }
    }
    last
}

/// `from` and `to` mixed at `t` in their encoded components.
fn mix(from: u64, to: u64, t: f32) -> u64 {
    let [red, green, blue, alpha] = lerp4(unpack(from), unpack(to), t);
    pack(red, green, blue, alpha)
}

/// The encoded components of an `EXTENDED_SRGB` `ColorLong`.
fn unpack(color: u64) -> [f32; 4] {
    let channel = |shift: u32| unhalf(((color >> shift) & 0xffff) as u16);
    [
        channel(48),
        channel(32),
        channel(16),
        f32::from(((color >> 6) & 0x3ff) as u16) / 1023.0,
    ]
}

/// The value of binary16 `bits`.
fn unhalf(bits: u16) -> f32 {
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let mantissa = f32::from(bits & 0x03ff);
    let magnitude = match exponent {
        0 => mantissa * 2f32.powi(-24),
        0x1f if mantissa == 0.0 => f32::INFINITY,
        0x1f => f32::NAN,
        _ => (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15),
    };
    sign * magnitude
}

/// Linear sRGB, premultiplied: the space a linear-light interpolation runs
/// in.
fn premultiplied_linear(color: WorkingColor) -> [f32; 4] {
    let [r, g, b, a] = linear_srgb(color);
    [r * a, g * a, b * a, a]
}

fn encode_premultiplied([r, g, b, a]: [f32; 4]) -> [f32; 4] {
    if a <= 0.0 {
        return [0.0; 4];
    }
    [
        srgb_encode(r / a),
        srgb_encode(g / a),
        srgb_encode(b / a),
        a,
    ]
}

fn lerp4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    std::array::from_fn(|i| (b[i] - a[i]).mul_add(t, a[i]))
}

/// Inserts the stops strictly between `(t0, c0)` and `(t1, c1)` (linear,
/// premultiplied) that keep the encoded interpolation within tolerance.
fn subdivide(
    t0: f32,
    c0: [f32; 4],
    t1: f32,
    c1: [f32; 4],
    depth: u32,
    colors: &mut Vec<u64>,
    positions: &mut Vec<f32>,
) {
    if depth == 0 || t1 <= t0 {
        return;
    }
    let mid = lerp4(c0, c1, 0.5);
    let exact = encode_premultiplied(mid);
    let approx = lerp4(encode_premultiplied(c0), encode_premultiplied(c1), 0.5);
    let error = exact
        .iter()
        .zip(approx)
        .map(|(e, a)| (e - a).abs())
        .fold(0.0f32, f32::max);
    if error <= GRADIENT_TOLERANCE {
        return;
    }
    let tm = f32::midpoint(t0, t1);
    subdivide(t0, c0, tm, mid, depth - 1, colors, positions);
    let [r, g, b, a] = exact;
    colors.push(pack(r, g, b, a));
    positions.push(tm);
    subdivide(tm, mid, t1, c1, depth - 1, colors, positions);
}

#[cfg(test)]
mod tests {
    use waterui_graphics::draw::color::WorkingColor;
    use waterui_graphics::draw::{ColorStop, Interpolation};

    use waterui_graphics::draw::Extend;

    use super::{MAX_SWEEP_STOPS, gradient_stops, half, pack, srgb_encode, sweep_turn};

    const RED: u64 = 0x3c00_0000_0000_ffc2;
    const BLUE: u64 = 0x0000_0000_3c00_ffc2;

    fn turn(k: f64, extend: Extend) -> Result<(Vec<u64>, Vec<f32>), usize> {
        let (mut colors, mut positions) = (Vec::new(), Vec::new());
        sweep_turn(
            k,
            extend,
            &[RED, BLUE],
            &[0.0, 1.0],
            &mut colors,
            &mut positions,
        )?;
        Ok((colors, positions))
    }

    #[test]
    fn a_sweep_over_two_turns_cuts_its_stops_halfway() {
        let (colors, positions) = turn(0.5, Extend::Pad).unwrap();
        assert_eq!(positions, [0.0, 1.0]);
        assert_eq!(colors[0], RED);
        assert_ne!(colors[1], BLUE);
    }

    #[test]
    fn a_half_turn_sweep_repeats_or_mirrors_its_stops() {
        let (colors, positions) = turn(2.0, Extend::Repeat).unwrap();
        assert_eq!(positions, [0.0, 0.0, 0.5, 0.5, 0.5, 0.5, 1.0, 1.0]);
        assert_eq!(colors, [RED, RED, BLUE, BLUE, RED, RED, BLUE, BLUE]);
        let (colors, positions) = turn(2.0, Extend::Reflect).unwrap();
        assert_eq!(positions, [0.0, 0.0, 0.5, 0.5, 0.5, 0.5, 1.0, 1.0]);
        assert_eq!(colors, [RED, RED, BLUE, BLUE, BLUE, BLUE, RED, RED]);
    }

    #[test]
    fn a_half_turn_sweep_pads_or_ends_after_its_span() {
        let (colors, positions) = turn(2.0, Extend::Pad).unwrap();
        assert_eq!((colors, positions), (vec![RED, BLUE], vec![0.0, 0.5]));
        let (colors, positions) = turn(2.0, Extend::None).unwrap();
        assert_eq!(positions, [0.0, 0.5, 0.5, 0.5, 1.0]);
        assert_eq!(colors[3] & 0xffc0, 0);
    }

    #[test]
    fn a_sweep_past_the_stop_budget_fails_with_its_count() {
        let cycles = MAX_SWEEP_STOPS;
        #[expect(
            clippy::cast_precision_loss,
            reason = "the budget is far below f64's integer range"
        )]
        let k = cycles as f64;
        assert_eq!(turn(k, Extend::Repeat), Err(cycles * 4));
    }

    #[test]
    fn halves_round_like_android_half() {
        assert_eq!(half(0.0), 0x0000);
        assert_eq!(half(-0.0), 0x8000);
        assert_eq!(half(1.0), 0x3c00);
        assert_eq!(half(-2.0), 0xc000);
        assert_eq!(half(0.5), 0x3800);
        assert_eq!(half(65504.0), 0x7bff);
        assert_eq!(half(1.0e6), 0x7c00);
        assert_eq!(half(f32::NAN) & 0x7e00, 0x7e00);
        // The smallest subnormal and a value exactly halfway to the next.
        assert_eq!(half(5.960_464_5e-8), 0x0001);
        assert_eq!(half(1.0 + 1.0 / 2048.0), 0x3c00);
        assert_eq!(half(1.0 + 3.0 / 2048.0), 0x3c02);
    }

    #[test]
    fn colors_pack_as_extended_srgb_color_longs() {
        // Opaque white: halves of 1.0, alpha 1023, space 2.
        assert_eq!(pack(1.0, 1.0, 1.0, 1.0), 0x3c00_3c00_3c00_ffc2);
        assert_eq!(pack(0.0, 0.0, 0.0, 0.0), 0x0000_0000_0000_0002);
    }

    #[test]
    fn the_transfer_is_mirrored_below_zero() {
        assert!((srgb_encode(1.0) - 1.0).abs() < 1e-6);
        assert!((srgb_encode(-0.5) + srgb_encode(0.5)).abs() < 1e-6);
        assert!(srgb_encode(4.0) > 1.0);
    }

    #[test]
    fn a_working_gradient_gains_stops_an_encoded_one_does_not() {
        let stops = [
            ColorStop {
                offset: 0.0,
                color: WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            },
            ColorStop {
                offset: 1.0,
                color: WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
            },
        ];
        let (mut colors, mut positions) = (Vec::new(), Vec::new());
        gradient_stops(
            &stops,
            Interpolation::SrgbEncoded,
            &mut colors,
            &mut positions,
        );
        assert_eq!(positions, [0.0, 1.0]);
        gradient_stops(&stops, Interpolation::Working, &mut colors, &mut positions);
        assert!(positions.len() > 2);
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(colors.len(), positions.len());
    }
}
