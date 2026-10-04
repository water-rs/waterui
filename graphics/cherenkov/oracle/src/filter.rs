//! Layer filters in `f64` on premultiplied linear Display P3 pixels,
//! written from the filter definitions in [`cherenkov_scene::LayerFilter`]:
//!
//! - colour matrices map premultiplied colour, the bias scaled by alpha;
//! - the Gaussian blur is the oracle's true Gaussian ([`gaussian_blur`]),
//!   per channel, edge-clamped;
//! - the box blur averages `2r + 1` edge-clamped texels per axis;
//! - the image blend operates on the input's unpremultiplied colour and the
//!   sampled texel, then re-premultiplies with the unchanged input alpha.

use cherenkov_scene::{FilterBlend, LayerFilter};

use crate::shadow::gaussian_blur;

/// Straight-alpha texels of a filter image, `channel / 255`, row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct Texels {
    /// Width in texels.
    pub width: usize,
    /// Height in texels.
    pub height: usize,
    /// Row-major RGBA.
    pub texels: Vec<[f64; 4]>,
}

/// Apply `filter` to the `width`×`height` premultiplied `pixels`.
/// `image` is the decoded image of a [`LayerFilter::BlendImage`].
///
/// # Panics
/// If a [`LayerFilter::BlendImage`] comes without `image`.
pub fn apply(
    filter: &LayerFilter,
    image: Option<&Texels>,
    pixels: &mut [[f64; 4]],
    width: usize,
    height: usize,
) {
    match filter {
        LayerFilter::ColorMatrix { matrix } => color_matrix(matrix, pixels),
        LayerFilter::ColorMatrixChain { first, second } => {
            color_matrix(first, pixels);
            color_matrix(second, pixels);
        }
        LayerFilter::GaussianBlur { sigma } => {
            for channel in 0..4 {
                let plane: Vec<f64> = pixels.iter().map(|p| p[channel]).collect();
                let blurred = gaussian_blur(&plane, width, height, *sigma);
                for (p, v) in pixels.iter_mut().zip(blurred) {
                    p[channel] = v;
                }
            }
        }
        LayerFilter::BoxBlur { radius } => box_blur(*radius, pixels, width, height),
        LayerFilter::BlendImage { amount, mode, .. } => {
            let image = image.expect("a blend-image filter carries its image");
            blend_image(image, amount.clamp(0.0, 1.0), *mode, pixels, width, height);
        }
    }
}

#[expect(
    clippy::many_single_char_names,
    clippy::suboptimal_flops,
    reason = "channel letters and the multiply-add order mirror the matrix-row formula"
)]
fn color_matrix(m: &[f64; 12], pixels: &mut [[f64; 4]]) {
    for p in pixels {
        let [r, g, b, a] = *p;
        let row = |i: usize| m[i] * r + m[i + 1] * g + m[i + 2] * b + m[i + 3] * a;
        *p = [row(0), row(4), row(8), a];
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    reason = "radii and pixel indices are small non-negative values"
)]
fn box_blur(radius: f64, pixels: &mut [[f64; 4]], width: usize, height: usize) {
    let r = radius.round().max(0.0) as i64;
    if r == 0 || pixels.is_empty() {
        return;
    }
    let taps = (2 * r + 1) as f64;
    let pass = |src: &[[f64; 4]], dx: i64, dy: i64| -> Vec<[f64; 4]> {
        let mut out = vec![[0.0; 4]; src.len()];
        for y in 0..height as i64 {
            for x in 0..width as i64 {
                let mut sum = [0.0; 4];
                for o in -r..=r {
                    let xx = (x + o * dx).clamp(0, width as i64 - 1) as usize;
                    let yy = (y + o * dy).clamp(0, height as i64 - 1) as usize;
                    let s = src[yy * width + xx];
                    for c in 0..4 {
                        sum[c] += s[c];
                    }
                }
                out[y as usize * width + x as usize] = sum.map(|v| v / taps);
            }
        }
        out
    };
    let horizontal = pass(pixels, 1, 0);
    let vertical = pass(&horizontal, 0, 1);
    pixels.copy_from_slice(&vertical);
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "texel indices are small non-negative values"
)]
#[expect(
    clippy::suboptimal_flops,
    reason = "the amount interpolation mirrors the blend lerp definition"
)]
fn blend_image(
    image: &Texels,
    amount: f64,
    mode: FilterBlend,
    pixels: &mut [[f64; 4]],
    width: usize,
    height: usize,
) {
    for y in 0..height {
        let ty = (((y as f64 + 0.5) / height as f64) * image.height as f64).floor() as usize;
        let ty = ty.min(image.height - 1);
        for x in 0..width {
            let tx = (((x as f64 + 0.5) / width as f64) * image.width as f64).floor() as usize;
            let tx = tx.min(image.width - 1);
            let t = image.texels[ty * image.width + tx];
            let p = &mut pixels[y * width + x];
            let alpha = p[3];
            if alpha <= 0.0 {
                continue;
            }
            let colour = [p[0] / alpha, p[1] / alpha, p[2] / alpha];
            let blended = blend(mode, colour, [t[0], t[1], t[2]]);
            for c in 0..3 {
                p[c] = (colour[c] * (1.0 - amount) + blended[c] * amount) * alpha;
            }
        }
    }
}

#[expect(
    clippy::suboptimal_flops,
    reason = "operation order mirrors the separable blend-mode formulas"
)]
fn blend(mode: FilterBlend, base: [f64; 3], top: [f64; 3]) -> [f64; 3] {
    let each = |f: fn(f64, f64) -> f64| std::array::from_fn(|c| f(base[c], top[c]));
    match mode {
        FilterBlend::Normal => top,
        FilterBlend::Multiply => each(|b, t| b * t),
        FilterBlend::Screen => each(|b, t| 1.0 - (1.0 - b) * (1.0 - t)),
        FilterBlend::Overlay => each(overlay),
        FilterBlend::Darken => each(f64::min),
        FilterBlend::Lighten => each(f64::max),
        FilterBlend::SoftLight => each(|b, t| {
            if t < 0.5 {
                b - (1.0 - 2.0 * t) * b * (1.0 - b)
            } else {
                b + (2.0 * t - 1.0) * (b.max(0.0).sqrt() - b)
            }
        }),
        FilterBlend::HardLight => each(|b, t| overlay(t, b)),
        FilterBlend::Difference => each(|b, t| (b - t).abs()),
        FilterBlend::Exclusion => each(|b, t| b + t - 2.0 * b * t),
        FilterBlend::ColorDodge => each(|b, t| b / (1.0 - t).max(1e-4)),
        FilterBlend::ColorBurn => each(|b, t| 1.0 - (1.0 - b) / t.max(1e-4)),
        FilterBlend::Hue => hsl_mix(base, top, [true, false, false]),
        FilterBlend::Saturation => hsl_mix(base, top, [false, true, false]),
        FilterBlend::Color => hsl_mix(base, top, [true, true, false]),
        FilterBlend::Luminosity => hsl_mix(base, top, [false, false, true]),
    }
}

/// `2·b·t` where `b < ½`, else `1 − 2(1 − b)(1 − t)`.
#[expect(
    clippy::suboptimal_flops,
    reason = "operation order mirrors the formula in the doc comment"
)]
fn overlay(b: f64, t: f64) -> f64 {
    if b < 0.5 {
        2.0 * b * t
    } else {
        1.0 - 2.0 * (1.0 - b) * (1.0 - t)
    }
}

/// Take each HSL component from `top` where `take` is set, else `base`.
fn hsl_mix(base: [f64; 3], top: [f64; 3], take: [bool; 3]) -> [f64; 3] {
    let (b, t) = (rgb_to_hsl(base), rgb_to_hsl(top));
    hsl_to_rgb(std::array::from_fn(|i| if take[i] { t[i] } else { b[i] }))
}

/// The hexcone HSL model; hue in `0..1`.
#[expect(clippy::float_cmp, reason = "exact channel ties select the hue sector")]
#[expect(
    clippy::many_single_char_names,
    reason = "r, g, b and the derived h, s, l are the hexcone channel names"
)]
fn rgb_to_hsl([r, g, b]: [f64; 3]) -> [f64; 3] {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = f64::midpoint(max, min);
    if max == min {
        return [0.0, 0.0, l];
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    [h / 6.0, s, l]
}

#[expect(
    clippy::many_single_char_names,
    clippy::suboptimal_flops,
    reason = "h, s, l, p, q and t are the hexcone-model names and the operation order mirrors the conversion"
)]
fn hsl_to_rgb([h, s, l]: [f64; 3]) -> [f64; 3] {
    if s == 0.0 {
        return [l; 3];
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let channel = |t: f64| {
        let t = if t < 0.0 {
            t + 1.0
        } else if t > 1.0 {
            t - 1.0
        } else {
            t
        };
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_matrix_scales_bias_by_alpha() {
        let mut px = [[0.1, 0.2, 0.3, 0.5]];
        let m = [
            0.0, 1.0, 0.0, 0.2, //
            1.0, 0.0, 0.0, 0.0, //
            0.0, 0.0, 2.0, -0.1,
        ];
        color_matrix(&m, &mut px);
        let expect = [0.2 + 0.1, 0.1, 0.6 - 0.05, 0.5];
        for c in 0..4 {
            assert!((px[0][c] - expect[c]).abs() < 1e-12, "{px:?}");
        }
    }

    #[test]
    fn box_blur_averages_clamped_taps() {
        let mut px = vec![[0.0; 4]; 5];
        px[0] = [1.0; 4];
        box_blur(1.0, &mut px, 5, 1);
        // Clamped row [1,1,0,0,0,0]: the first two outputs see two ones.
        assert!((px[0][0] - 2.0 / 3.0).abs() < 1e-12);
        assert!((px[1][0] - 1.0 / 3.0).abs() < 1e-12);
        assert!(px[2][0].abs() < 1e-12);
    }

    #[test]
    fn hsl_round_trips() {
        for rgb in [
            [0.2, 0.5, 0.9],
            [0.9, 0.1, 0.4],
            [0.3, 0.3, 0.3],
            [0.7, 0.6, 0.1],
        ] {
            let back = hsl_to_rgb(rgb_to_hsl(rgb));
            for c in 0..3 {
                assert!((back[c] - rgb[c]).abs() < 1e-12, "{rgb:?} -> {back:?}");
            }
        }
    }

    #[test]
    #[expect(
        clippy::suboptimal_flops,
        reason = "the tolerance is spelled as relative-plus-absolute, not fused"
    )]
    fn image_blend_unpremultiplies_and_preserves_transparent_pixels() {
        let alpha = 1.0e-8;
        let colour = [0.6, 0.1, 0.1];
        let image = Texels {
            width: 1,
            height: 1,
            texels: vec![[240.0 / 255.0, 36.0 / 255.0, 48.0 / 255.0, 1.0]],
        };
        let mut pixels = [
            [0.0; 4],
            [colour[0], colour[1], colour[2], 1.0],
            [
                colour[0] * alpha,
                colour[1] * alpha,
                colour[2] * alpha,
                alpha,
            ],
        ];

        blend_image(&image, 1.0, FilterBlend::Luminosity, &mut pixels, 3, 1);

        assert_eq!(pixels[0].map(f64::to_bits), [0; 4]);
        assert_eq!(pixels[1][3].to_bits(), 1.0_f64.to_bits());
        assert_eq!(pixels[2][3].to_bits(), alpha.to_bits());
        for (channel, full) in pixels[1][..3].iter().enumerate() {
            let expected = *full * alpha;
            let tolerance = expected.abs() * 1.0e-12 + 1.0e-20;
            assert!(
                (pixels[2][channel] - expected).abs() <= tolerance,
                "channel {channel}: expected {expected}, got {}",
                pixels[2][channel]
            );
        }
    }
}
