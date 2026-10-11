//! Colour spaces: the working space's constants, a stage's operating
//! space, and the working-space ↔ sRGB conversion that brackets a stage
//! in its declared space.
//!
//! `powf` is the platform's `f32::powf` when the default `std` feature is
//! on, `libm`'s without it (`no_std` targets); the rest of the conversion
//! is plain multiply-add arithmetic, as the WGSL stages do not guarantee
//! fused operations.

/// The colour space a stage operates in.
///
/// Executors hand every stage premultiplied colours in the space it declares
/// and convert around it. Extended values (negative components, values above
/// one) stay extended through every conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OperatingSpace {
    /// The linear working space, linear Display P3.
    #[default]
    Working,
    /// sRGB: sRGB primaries with the sRGB transfer function applied, the
    /// space CSS filter functions are defined in.
    Srgb,
}

/// The working-space constants a stage receives through its `space`
/// argument, so filters never hard-code another space's coefficients.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkingSpace {
    /// The luma coefficients: the Y row of the working space's RGB to XYZ
    /// matrix.
    pub luma: [f32; 3],
}

impl WorkingSpace {
    /// Linear Display P3 (D65), the working space.
    pub const LINEAR_DISPLAY_P3: Self = Self {
        luma: [0.228_974_6, 0.691_738_5, 0.079_286_9],
    };
}

/// Linear Display P3 to linear sRGB (both D65), rows first: the matrix of
/// the `to_srgb` conversion stage (`filtrate`'s
/// `shaders/space/to_srgb.wgsl`).
pub const P3_TO_SRGB: [[f32; 3]; 3] = [
    [1.224_940_2, -0.224_940_2, 0.0],
    [-0.042_057, 1.042_057, 0.0],
    [-0.019_637_6, -0.078_636, 1.098_273_6],
];

/// Linear sRGB to linear Display P3 (both D65), rows first: the matrix of
/// the `from_srgb` conversion stage (`filtrate`'s
/// `shaders/space/from_srgb.wgsl`).
pub const SRGB_TO_P3: [[f32; 3]; 3] = [
    [0.822_462, 0.177_538, 0.0],
    [0.033_194_2, 0.966_805_8, 0.0],
    [0.017_082_6, 0.072_397_4, 0.910_519_9],
];

/// The `to_srgb` conversion stage on the CPU: premultiplied working-space
/// pixels to premultiplied sRGB, the transfer mirrored through zero so
/// extended values stay extended.
pub fn to_srgb(pixels: &mut [[f32; 4]]) {
    convert(pixels, |straight| {
        transform(&P3_TO_SRGB, straight).map(srgb_encode)
    });
}

/// The `from_srgb` conversion stage on the CPU: premultiplied sRGB back to
/// premultiplied working-space pixels.
pub fn from_srgb(pixels: &mut [[f32; 4]]) {
    convert(pixels, |straight| {
        transform(&SRGB_TO_P3, straight.map(srgb_decode))
    });
}

/// `x` to the power of `n` — `f32::powf` under `std`, `libm::powf` on
/// `no_std` targets.
#[cfg(feature = "std")]
#[inline]
fn powf(x: f32, n: f32) -> f32 {
    x.powf(n)
}

/// `x` to the power of `n` — `f32::powf` under `std`, `libm::powf` on
/// `no_std` targets.
#[cfg(not(feature = "std"))]
#[inline]
fn powf(x: f32, n: f32) -> f32 {
    libm::powf(x, n)
}

/// The sRGB transfer of one linear channel, mirrored through zero.
#[must_use]
#[allow(
    clippy::suboptimal_flops,
    reason = "the WGSL does not guarantee fused operations; plain multiply-add keeps per-pixel loops vectorisable"
)]
pub fn srgb_encode(linear: f32) -> f32 {
    let magnitude = linear.abs();
    let curve = if magnitude > 0.003_130_8 {
        1.055 * powf(magnitude, 1.0 / 2.4) - 0.055
    } else {
        magnitude * 12.92
    };
    curve.copysign(linear)
}

/// The inverse of [`srgb_encode`]: one encoded channel back to linear,
/// mirrored through zero.
#[must_use]
pub fn srgb_decode(encoded: f32) -> f32 {
    let magnitude = encoded.abs();
    let curve = if magnitude > 0.040_45 {
        powf((magnitude + 0.055) / 1.055, 2.4)
    } else {
        magnitude / 12.92
    };
    curve.copysign(encoded)
}

/// Applies `map` to each pixel's straight-alpha colour, as the conversion
/// stages do: the colour is unpremultiplied by `max(alpha, 1e-6)` and
/// premultiplied again by alpha.
pub fn convert(pixels: &mut [[f32; 4]], map: impl Fn([f32; 3]) -> [f32; 3]) {
    for pixel in pixels {
        let alpha = pixel[3];
        let divisor = alpha.max(1.0e-6);
        let [r, g, b] = map([pixel[0] / divisor, pixel[1] / divisor, pixel[2] / divisor]);
        *pixel = [r * alpha, g * alpha, b * alpha, alpha];
    }
}

/// `matrix * rgb`, `matrix` given rows first.
#[must_use]
#[allow(
    clippy::suboptimal_flops,
    reason = "the WGSL does not guarantee fused operations; plain multiply-add keeps per-pixel loops vectorisable"
)]
pub fn transform(matrix: &[[f32; 3]; 3], rgb: [f32; 3]) -> [f32; 3] {
    matrix.map(|row| row[0] * rgb[0] + row[1] * rgb[1] + row[2] * rgb[2])
}
