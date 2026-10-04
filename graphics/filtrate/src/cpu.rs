//! SIMD CPU kernels for colour filters.
//!
//! Every kernel here is a linear map on premultiplied RGBA, so each builds a
//! 4x4 matrix from its parameters and runs the shared SIMD loop: one `f32x4`
//! per pixel, the output accumulated column by column with fused
//! multiply-adds.

use filtrate_core::{
    AuxData, AuxFormat, AuxImage, CpuFilter, CpuFilterError, CpuImage, FilterParam, SpatialFilter,
    WorkingSpace,
};
use wide::f32x4;

use crate::filters::{BlendWithImage, Blur, GAUSSIAN_RADIUS_PER_SIGMA, GaussianBlur};

/// A 4x4 matrix on premultiplied RGBA, stored as columns: the output is
/// `c[0] * r + c[1] * g + c[2] * b + c[3] * a`.
struct Matrix([f32x4; 4]);

impl Matrix {
    fn apply(&self, pixels: &mut [[f32; 4]]) {
        let [red, green, blue, alpha] = self.0;
        for pixel in pixels {
            let [r, g, b, a] = *pixel;
            let out = alpha * f32x4::splat(a);
            let out = blue.mul_add(f32x4::splat(b), out);
            let out = green.mul_add(f32x4::splat(g), out);
            *pixel = red.mul_add(f32x4::splat(r), out).to_array();
        }
    }

    /// `rgb' = keep * rgb + toward * luma(rgb)`, alpha unchanged: the shape
    /// of saturation and grayscale.
    fn luma_mix(space: &WorkingSpace, keep: f32, toward: f32) -> Self {
        let column = |channel: usize| {
            let mut column = [toward * space.luma[channel]; 4];
            column[channel] += keep;
            column[3] = 0.0;
            f32x4::new(column)
        };
        Self([
            column(0),
            column(1),
            column(2),
            f32x4::new([0.0, 0.0, 0.0, 1.0]),
        ])
    }
}

/// [`Brightness`](crate::filters::Brightness): `rgb + amount * a`.
pub fn brightness(params: [f32; 1], _space: &WorkingSpace, pixels: &mut [[f32; 4]]) {
    let amount = params[0];
    Matrix([
        f32x4::new([1.0, 0.0, 0.0, 0.0]),
        f32x4::new([0.0, 1.0, 0.0, 0.0]),
        f32x4::new([0.0, 0.0, 1.0, 0.0]),
        f32x4::new([amount, amount, amount, 1.0]),
    ])
    .apply(pixels);
}

/// [`Saturation`](crate::filters::Saturation): `mix(luma, rgb, amount)`.
pub fn saturation(params: [f32; 1], space: &WorkingSpace, pixels: &mut [[f32; 4]]) {
    let amount = params[0];
    Matrix::luma_mix(space, amount, 1.0 - amount).apply(pixels);
}

/// [`Grayscale`](crate::filters::Grayscale): `mix(rgb, luma, intensity)`.
pub fn grayscale(params: [f32; 1], space: &WorkingSpace, pixels: &mut [[f32; 4]]) {
    let intensity = params[0];
    Matrix::luma_mix(space, 1.0 - intensity, intensity).apply(pixels);
}

/// [`HueRotation`](crate::filters::HueRotation): the CSS/SVG `hue-rotate`
/// matrix (Filter Effects Module Level 1, `feColorMatrix
/// type="hueRotate"`) on premultiplied RGB, alpha unchanged.
pub fn hue_rotation(params: [f32; 1], _space: &WorkingSpace, pixels: &mut [[f32; 4]]) {
    let (sin, cos) = params[0].to_radians().sin_cos();
    // A spec coefficient: `base + cos·cos_k + sin·sin_k`.
    let coefficient =
        |base: f32, cos_k: f32, sin_k: f32| sin.mul_add(sin_k, cos.mul_add(cos_k, base));
    Matrix([
        // Columns are the red, green and blue input channels' contributions
        // to (r', g', b').
        f32x4::new([
            coefficient(0.213, 0.787, -0.213),
            coefficient(0.213, -0.213, 0.143),
            coefficient(0.213, -0.213, -0.787),
            0.0,
        ]),
        f32x4::new([
            coefficient(0.715, -0.715, -0.715),
            coefficient(0.715, 0.285, 0.140),
            coefficient(0.715, -0.715, 0.715),
            0.0,
        ]),
        f32x4::new([
            coefficient(0.072, -0.072, 0.928),
            coefficient(0.072, -0.072, -0.283),
            coefficient(0.072, 0.928, 0.072),
            0.0,
        ]),
        f32x4::new([0.0, 0.0, 0.0, 1.0]),
    ])
    .apply(pixels);
}

/// [`ColorMatrix`](crate::filters::ColorMatrix): the 3x4 matrix on
/// straight-alpha RGB, whose bias column scales with alpha on premultiplied
/// colour.
pub fn color_matrix(params: [f32; 12], _space: &WorkingSpace, pixels: &mut [[f32; 4]]) {
    let column = |index: usize, alpha: f32| {
        f32x4::new([params[index], params[4 + index], params[8 + index], alpha])
    };
    Matrix([
        column(0, 0.0),
        column(1, 0.0),
        column(2, 0.0),
        column(3, 1.0),
    ])
    .apply(pixels);
}

impl<T: FilterParam> CpuFilter for GaussianBlur<T> {
    fn cpu_footprint(params: &Self::Params) -> filtrate_core::Footprint {
        Self::footprint_of(params)
    }

    fn apply_cpu_image(
        &self,
        params: &Self::Params,
        _space: &WorkingSpace,
        image: &mut CpuImage<'_>,
    ) -> Result<(), CpuFilterError> {
        gaussian_blur(params[0], image);
        Ok(())
    }
}

impl<T: FilterParam> CpuFilter for Blur<T> {
    fn cpu_footprint(params: &Self::Params) -> filtrate_core::Footprint {
        Self::footprint_of(params)
    }

    fn apply_cpu_image(
        &self,
        params: &Self::Params,
        _space: &WorkingSpace,
        image: &mut CpuImage<'_>,
    ) -> Result<(), CpuFilterError> {
        box_blur(params[0], image);
        Ok(())
    }
}

impl<A: FilterParam> CpuFilter for BlendWithImage<A> {
    fn cpu_footprint(params: &Self::Params) -> filtrate_core::Footprint {
        Self::footprint_of(params)
    }

    fn apply_cpu_image(
        &self,
        params: &Self::Params,
        _space: &WorkingSpace,
        image: &mut CpuImage<'_>,
    ) -> Result<(), CpuFilterError> {
        let data = self
            .image
            .data()
            .ok_or(CpuFilterError::GpuImage { index: 0 })?;
        blend_with_image(
            *params,
            data,
            self.image.width(),
            self.image.height(),
            image,
        );
        Ok(())
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the positive WGSL radius is converted to the CPU loop index"
)]
fn gaussian_blur(sigma: f32, image: &mut CpuImage<'_>) {
    let width = image.size.0;
    if width == 0 || image.pixels.is_empty() {
        return;
    }
    let height = image.pixels.len() / width;
    debug_assert_eq!(height * width, image.pixels.len());
    let sigma = sigma.max(0.001);
    let radius = (sigma * GAUSSIAN_RADIUS_PER_SIGMA).ceil() as usize;
    let (weights, weight_total) = gaussian_weights(sigma, radius);
    let mut temporary = vec![[0.0; 4]; image.pixels.len()];
    gaussian_pass(
        image.pixels,
        &mut temporary,
        width,
        height,
        false,
        &weights,
        weight_total,
    );
    gaussian_pass(
        &temporary,
        image.pixels,
        width,
        height,
        true,
        &weights,
        weight_total,
    );
}

#[expect(
    clippy::cast_precision_loss,
    clippy::suboptimal_flops,
    reason = "the blur loop mirrors WGSL f32 offsets and accumulation order"
)]
fn gaussian_weights(sigma: f32, radius: usize) -> (Vec<f32>, f32) {
    let k = 1.0 / (sigma * std::f32::consts::SQRT_2);
    let mut edge = gaussian_erf(0.5 * k);
    let mut weights = Vec::with_capacity(radius + 1);
    weights.push(edge);
    let mut total = edge;
    for offset in 1..=radius {
        let next_edge = gaussian_erf((offset as f32 + 0.5) * k);
        let weight = 0.5 * (next_edge - edge);
        weights.push(weight);
        total += 2.0 * weight;
        edge = next_edge;
    }
    (weights, total)
}

#[expect(
    clippy::suboptimal_flops,
    reason = "matches the Abramowitz and Stegun 7.1.26 WGSL evaluation order"
)]
fn gaussian_erf(x: f32) -> f32 {
    let sign = x.signum();
    let a = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * a);
    let y = 1.0
        - (((((1.061_405_4 * t - 1.453_152_1) * t + 1.421_413_8) * t - 0.284_496_72) * t
            + 0.254_829_6)
            * t
            * (-a * a).exp());
    sign * y
}

#[expect(
    clippy::suboptimal_flops,
    reason = "the per-pixel accumulation order mirrors WGSL"
)]
fn gaussian_pass(
    input: &[[f32; 4]],
    output: &mut [[f32; 4]],
    width: usize,
    height: usize,
    vertical: bool,
    weights: &[f32],
    weight_total: f32,
) {
    for y in 0..height {
        for x in 0..width {
            let center = y * width + x;
            let mut sum = input[center].map(|channel| channel * weights[0]);
            for (offset, weight) in weights.iter().enumerate().skip(1) {
                let (minus, plus) = if vertical {
                    let y0 = y.saturating_sub(offset) * width + x;
                    let y1 = y.saturating_add(offset).min(height - 1) * width + x;
                    (input[y0], input[y1])
                } else {
                    let x0 = x.saturating_sub(offset);
                    let x1 = x.saturating_add(offset).min(width - 1);
                    (input[y * width + x0], input[y * width + x1])
                };
                for channel in 0..4 {
                    sum[channel] += (minus[channel] + plus[channel]) * *weight;
                }
            }
            for channel in &mut sum {
                *channel /= weight_total;
            }
            output[center] = sum;
        }
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the nonnegative WGSL radius is converted to the CPU loop index"
)]
fn box_blur(radius: f32, image: &mut CpuImage<'_>) {
    let width = image.size.0;
    if width == 0 || image.pixels.is_empty() {
        return;
    }
    let height = image.pixels.len() / width;
    debug_assert_eq!(height * width, image.pixels.len());
    let radius = radius.round().max(0.0) as usize;
    if radius == 0 {
        return;
    }
    let mut temporary = vec![[0.0; 4]; image.pixels.len()];
    box_pass(image.pixels, &mut temporary, width, height, radius, false);
    box_pass(&temporary, image.pixels, width, height, radius, true);
}

#[expect(
    clippy::cast_precision_loss,
    reason = "blur radii are bounded by the Raster surface dimensions"
)]
fn box_pass(
    input: &[[f32; 4]],
    output: &mut [[f32; 4]],
    width: usize,
    height: usize,
    radius: usize,
    vertical: bool,
) {
    let divisor = radius.saturating_mul(2).saturating_add(1) as f32;
    for y in 0..height {
        for x in 0..width {
            let center = y * width + x;
            let mut sum = input[center];
            for offset in 1..=radius {
                let (minus, plus) = if vertical {
                    let y0 = y.saturating_sub(offset) * width + x;
                    let y1 = y.saturating_add(offset).min(height - 1) * width + x;
                    (input[y0], input[y1])
                } else {
                    let x0 = x.saturating_sub(offset);
                    let x1 = x.saturating_add(offset).min(width - 1);
                    (input[y * width + x0], input[y * width + x1])
                };
                for channel in 0..4 {
                    sum[channel] += minus[channel] + plus[channel];
                }
            }
            for channel in &mut sum {
                *channel /= divisor;
            }
            output[center] = sum;
        }
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "WGSL samples and mode selectors use f32 and bounded Raster dimensions"
)]
#[expect(
    clippy::suboptimal_flops,
    reason = "preserves the WGSL blend interpolation order"
)]
fn blend_with_image(
    params: [f32; 2],
    data: AuxData<'_>,
    aux_width: u32,
    aux_height: u32,
    image: &mut CpuImage<'_>,
) {
    let width = image.size.0;
    let height = image.pixels.len().checked_div(width).unwrap_or(0);
    if width == 0 || height == 0 {
        return;
    }
    let amount = params[0].clamp(0.0, 1.0);
    let mode = (params[1] + 0.5) as u32;
    for y in 0..height {
        let v = (image.top + y) as f32 + 0.5;
        let aux_y = ((v / image.size.1 as f32) * aux_height as f32)
            .floor()
            .clamp(0.0, aux_height.saturating_sub(1) as f32) as usize;
        for x in 0..width {
            let u = x as f32 + 0.5;
            let aux_x = ((u / width as f32) * aux_width as f32)
                .floor()
                .clamp(0.0, aux_width.saturating_sub(1) as f32) as usize;
            let index = aux_y * aux_width as usize + aux_x;
            let top = aux_pixel(data, index);
            let base = image.pixels[y * width + x];
            if base[3] <= 0.0 {
                continue;
            }
            let colour = [base[0] / base[3], base[1] / base[3], base[2] / base[3]];
            let blended = blend_color(colour, [top[0], top[1], top[2]], mode);
            for channel in 0..3 {
                image.pixels[y * width + x][channel] =
                    (colour[channel] * (1.0 - amount) + blended[channel] * amount) * base[3];
            }
            image.pixels[y * width + x][3] = base[3];
        }
    }
}

fn aux_pixel(data: AuxData<'_>, index: usize) -> [f32; 4] {
    match data.format {
        AuxFormat::Rgba8 => {
            let start = index * 4;
            std::array::from_fn(|channel| f32::from(data.bytes[start + channel]) / 255.0)
        }
        AuxFormat::Rgba16Float => {
            let start = index * 8;
            std::array::from_fn(|channel| {
                let byte = start + channel * 2;
                half::f16::from_bits(u16::from_le_bytes([data.bytes[byte], data.bytes[byte + 1]]))
                    .to_f32()
            })
        }
        AuxFormat::Rgba32Float => {
            let start = index * 16;
            std::array::from_fn(|channel| {
                let byte = start + channel * 4;
                f32::from_le_bytes([
                    data.bytes[byte],
                    data.bytes[byte + 1],
                    data.bytes[byte + 2],
                    data.bytes[byte + 3],
                ])
            })
        }
    }
}

#[expect(clippy::suboptimal_flops, reason = "preserves the WGSL blend formulas")]
fn blend_color(base: [f32; 3], top: [f32; 3], mode: u32) -> [f32; 3] {
    match mode {
        1 => std::array::from_fn(|i| base[i] * top[i]),
        2 => std::array::from_fn(|i| 1.0 - (1.0 - base[i]) * (1.0 - top[i])),
        3 => blend_overlay(base, top),
        4 => std::array::from_fn(|i| base[i].min(top[i])),
        5 => std::array::from_fn(|i| base[i].max(top[i])),
        6 => blend_soft_light(base, top),
        7 => blend_overlay(top, base),
        8 => std::array::from_fn(|i| (base[i] - top[i]).abs()),
        9 => std::array::from_fn(|i| base[i] + top[i] - 2.0 * base[i] * top[i]),
        10 => std::array::from_fn(|i| base[i] / (1.0 - top[i]).max(0.0001)),
        11 => std::array::from_fn(|i| 1.0 - (1.0 - base[i]) / top[i].max(0.0001)),
        12 => blend_hsl(base, top, true, false, false),
        13 => blend_hsl(base, top, false, true, false),
        14 => blend_hsl(base, top, true, true, false),
        15 => blend_hsl(base, top, false, false, true),
        _ => top,
    }
}

#[expect(
    clippy::suboptimal_flops,
    reason = "preserves the WGSL overlay formula"
)]
fn blend_overlay(base: [f32; 3], top: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| {
        if base[i] < 0.5 {
            2.0 * base[i] * top[i]
        } else {
            1.0 - 2.0 * (1.0 - base[i]) * (1.0 - top[i])
        }
    })
}

#[expect(
    clippy::suboptimal_flops,
    reason = "preserves the WGSL soft-light formula"
)]
fn blend_soft_light(base: [f32; 3], top: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| {
        if top[i] < 0.5 {
            base[i] - (1.0 - 2.0 * top[i]) * base[i] * (1.0 - base[i])
        } else {
            base[i] + (2.0 * top[i] - 1.0) * (base[i].max(0.0).sqrt() - base[i])
        }
    })
}

fn blend_hsl(
    base: [f32; 3],
    top: [f32; 3],
    hue_from_top: bool,
    saturation_from_top: bool,
    lightness_from_top: bool,
) -> [f32; 3] {
    let base_hsl = rgb_to_hsl(base);
    let top_hsl = rgb_to_hsl(top);
    hsl_to_rgb([
        if hue_from_top {
            top_hsl[0]
        } else {
            base_hsl[0]
        },
        if saturation_from_top {
            top_hsl[1]
        } else {
            base_hsl[1]
        },
        if lightness_from_top {
            top_hsl[2]
        } else {
            base_hsl[2]
        },
    ])
}

#[expect(
    clippy::float_cmp,
    reason = "the branch matches the WGSL HSL implementation exactly"
)]
fn rgb_to_hsl(rgb: [f32; 3]) -> [f32; 3] {
    let max = rgb[0].max(rgb[1]).max(rgb[2]);
    let min = rgb[0].min(rgb[1]).min(rgb[2]);
    let lightness = f32::midpoint(max, min);
    if max == min {
        return [0.0, 0.0, lightness];
    }
    let delta = max - min;
    let saturation = if lightness > 0.5 {
        delta / (2.0 - max - min)
    } else {
        delta / (max + min)
    };
    let hue = if max == rgb[0] {
        (rgb[1] - rgb[2]) / delta + if rgb[1] < rgb[2] { 6.0 } else { 0.0 }
    } else if max == rgb[1] {
        (rgb[2] - rgb[0]) / delta + 2.0
    } else {
        (rgb[0] - rgb[1]) / delta + 4.0
    } / 6.0;
    [hue, saturation, lightness]
}

#[expect(
    clippy::suboptimal_flops,
    reason = "preserves the WGSL hue conversion order"
)]
fn hue_to_rgb(p: f32, q: f32, mut t: f32) -> f32 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        p + (q - p) * 6.0 * t
    } else if t < 0.5 {
        q
    } else if t < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
        p
    }
}

#[expect(
    clippy::suboptimal_flops,
    reason = "preserves the WGSL HSL conversion order"
)]
fn hsl_to_rgb(hsl: [f32; 3]) -> [f32; 3] {
    if hsl[1] == 0.0 {
        return [hsl[2]; 3];
    }
    let q = if hsl[2] < 0.5 {
        hsl[2] * (1.0 + hsl[1])
    } else {
        hsl[2] + hsl[1] - hsl[2] * hsl[1]
    };
    let p = 2.0 * hsl[2] - q;
    [
        hue_to_rgb(p, q, hsl[0] + 1.0 / 3.0),
        hue_to_rgb(p, q, hsl[0]),
        hue_to_rgb(p, q, hsl[0] - 1.0 / 3.0),
    ]
}

#[cfg(test)]
mod tests {
    use super::{
        GAUSSIAN_RADIUS_PER_SIGMA, gaussian_blur, gaussian_weights, hsl_to_rgb, rgb_to_hsl,
    };
    use crate::FilterImage;
    use crate::filters::{BlendMode, BlendWithImage};
    use filtrate_core::{CpuFilter, CpuImage, Filter, WorkingSpace};

    #[expect(
        clippy::suboptimal_flops,
        reason = "the reference follows the Abramowitz and Stegun 7.1.26 evaluation order"
    )]
    fn erf_reference(x: f64) -> f64 {
        let sign = x.signum();
        let a = x.abs();
        let t = 1.0 / (1.0 + 0.327_591_1 * a);
        let y = 1.0
            - (((((1.061_405_429 * t - 1.453_152_027) * t + 1.421_413_741) * t - 0.284_496_736)
                * t
                + 0.254_829_592)
                * t
                * (-a * a).exp());
        sign * y
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        clippy::suboptimal_flops,
        reason = "fixed positive test sigmas keep reference radii and offsets bounded"
    )]
    #[test]
    fn gaussian_weights_match_integrated_kernel() {
        for sigma in [0.3_f32, 0.6, 1.7, 4.0, 12.0] {
            let radius = (sigma * GAUSSIAN_RADIUS_PER_SIGMA).ceil() as usize;
            let (weights, total) = gaussian_weights(sigma, radius);
            let k = 1.0 / (f64::from(sigma) * std::f64::consts::SQRT_2);
            let mut edge = erf_reference(0.5 * k);
            let mut reference = vec![edge];
            for offset in 1..=radius {
                let next_edge = erf_reference((offset as f64 + 0.5) * k);
                reference.push(0.5 * (next_edge - edge));
                edge = next_edge;
            }

            for (offset, (actual, expected)) in weights.iter().zip(reference).enumerate() {
                assert!(
                    (f64::from(*actual) - expected).abs() <= 2.0e-6,
                    "sigma {sigma}, offset {offset}: expected {expected}, got {actual}"
                );
            }

            let normalized_sum = (weights[0] + 2.0 * weights.iter().skip(1).sum::<f32>()) / total;
            assert!(
                (normalized_sum - 1.0).abs() <= 1.0e-6,
                "sigma {sigma}: normalized sum is {normalized_sum}"
            );
        }
    }

    #[test]
    fn gaussian_blur_at_minimum_sigma_is_identity() {
        let expected = [
            [0.25, 0.5, 0.75, 1.0],
            [2.0, 1.0, 0.5, 1.0],
            [0.0, 0.25, 0.125, 0.5],
            [4.0, 3.0, 2.0, 1.0],
        ];
        let mut pixels = expected;
        gaussian_blur(
            0.001,
            &mut CpuImage {
                pixels: &mut pixels,
                top: 0,
                size: (2, 2),
            },
        );
        assert_eq!(pixels, expected);
    }

    fn apply_blend(mode: BlendMode, pixels: &mut [[f32; 4]]) {
        let filter = BlendWithImage {
            image: FilterImage::from_rgba8(1, 1, vec![240, 36, 48, 255]),
            amount: 1.0_f32,
            mode,
        };
        let params = filter.params();
        let size = (pixels.len(), 1);
        filter
            .apply_cpu_image(
                &params,
                &WorkingSpace::LINEAR_DISPLAY_P3,
                &mut CpuImage {
                    pixels,
                    top: 0,
                    size,
                },
            )
            .expect("CPU image blend should succeed");
    }

    #[test]
    fn hsl_saturation_uses_the_lightness_branch_and_round_trips() {
        let light = [0.9, 0.6, 0.5];
        let light_hsl = rgb_to_hsl(light);
        assert!(
            (light_hsl[1] - 0.4 / 0.6).abs() < 1.0e-6,
            "unexpected light saturation: {}",
            light_hsl[1]
        );

        let dark_hsl = rgb_to_hsl([0.4, 0.2, 0.1]);
        assert!(
            (dark_hsl[1] - 0.3 / 0.5).abs() < 1.0e-6,
            "unexpected dark saturation: {}",
            dark_hsl[1]
        );

        let round_trip = hsl_to_rgb(light_hsl);
        for channel in 0..3 {
            assert!(
                (round_trip[channel] - light[channel]).abs() < 1.0e-6,
                "channel {channel}: expected {}, got {}",
                light[channel],
                round_trip[channel]
            );
        }
    }

    #[test]
    fn image_blend_uses_unpremultiplied_colour_for_all_modes() {
        let modes = [
            BlendMode::Normal,
            BlendMode::Multiply,
            BlendMode::Screen,
            BlendMode::Overlay,
            BlendMode::Darken,
            BlendMode::Lighten,
            BlendMode::SoftLight,
            BlendMode::HardLight,
            BlendMode::Difference,
            BlendMode::Exclusion,
            BlendMode::ColorDodge,
            BlendMode::ColorBurn,
            BlendMode::Hue,
            BlendMode::Saturation,
            BlendMode::Color,
            BlendMode::Luminosity,
        ];
        let colour = [0.6_f32, 0.1, 0.1];
        let top = [240.0_f32 / 255.0, 36.0 / 255.0, 48.0 / 255.0];
        let low_alpha = 1.0e-8_f32;
        let mid_alpha = 0.3_f32;

        for mode in modes {
            let mut pixels = [
                [0.0; 4],
                [
                    colour[0] * low_alpha,
                    colour[1] * low_alpha,
                    colour[2] * low_alpha,
                    low_alpha,
                ],
                [
                    colour[0] * mid_alpha,
                    colour[1] * mid_alpha,
                    colour[2] * mid_alpha,
                    mid_alpha,
                ],
                [colour[0], colour[1], colour[2], 1.0],
            ];
            apply_blend(mode, &mut pixels);

            assert_eq!(pixels[0].map(f32::to_bits), [0; 4], "{mode:?}");
            for (index, alpha) in [(1, low_alpha), (2, mid_alpha)] {
                for (channel, full) in pixels[3][..3].iter().enumerate() {
                    let expected = *full * alpha;
                    let tolerance = expected.abs().mul_add(1.0e-5, 1.0e-12);
                    assert!(
                        (pixels[index][channel] - expected).abs() <= tolerance,
                        "{mode:?}, alpha {alpha}, channel {channel}: expected {expected}, got {}",
                        pixels[index][channel]
                    );
                }
                assert_eq!(pixels[index][3].to_bits(), alpha.to_bits(), "{mode:?}");
            }

            if mode == BlendMode::Multiply {
                for (channel, base) in colour.iter().enumerate() {
                    let expected = base * top[channel];
                    assert!(
                        (pixels[3][channel] - expected).abs() <= 1.0e-6,
                        "Multiply, channel {channel}: expected {expected}, got {}",
                        pixels[3][channel]
                    );
                }
            }
        }
    }
}
