//! Colour spaces: the working space's constants, a stage's operating
//! space, and the working-space ↔ sRGB conversion that brackets a stage
//! in its declared space.
//!
//! The numbers — the matrices between linear Display P3 and linear sRGB,
//! the working space's luma row and the sRGB transfer parameters — have
//! one definition, `space/constants.rs`. The CPU conversion here uses it
//! directly, and the build script renders it into [`WGSL`], the fragment
//! every shader that converts between the spaces is composed with.
//!
//! `powf` is the platform's `f32::powf` when the default `std` feature is
//! on, `libm`'s without it (`no_std` targets); the rest of the conversion
//! is plain multiply-add arithmetic, as the WGSL stages do not guarantee
//! fused operations.

mod constants;

pub use constants::{
    P3_LUMA, P3_TO_SRGB, SRGB_ENCODED_BREAK, SRGB_GAMMA, SRGB_INVERSE_GAMMA, SRGB_LINEAR_BREAK,
    SRGB_OFFSET, SRGB_SCALE, SRGB_SLOPE, SRGB_TO_P3,
};

/// The WGSL fragment of the working-space ↔ sRGB conversion, rendered from
/// the same constants as the CPU functions of this module.
///
/// It declares `SPACE_P3_TO_SRGB` and `SPACE_SRGB_TO_P3` (`mat3x3<f32>`),
/// the transfer constants `SPACE_SRGB_*`, and the functions
///
/// - `space_srgb_encode(linear: vec3<f32>) -> vec3<f32>` and
///   `space_srgb_decode(encoded: vec3<f32>) -> vec3<f32>`, the transfer
///   mirrored through zero ([`srgb_encode`], [`srgb_decode`]);
/// - `space_to_srgb(color: vec4<f32>) -> vec4<f32>` and
///   `space_from_srgb(color: vec4<f32>) -> vec4<f32>`, the premultiplied
///   conversions ([`to_srgb`], [`from_srgb`]).
///
/// A shader prepends it to its own source.
pub const WGSL: &str = include_str!(concat!(env!("OUT_DIR"), "/space.wgsl"));

/// The `to_srgb` conversion stage: [`WGSL`] and an `apply` that converts
/// premultiplied working-space colour to premultiplied sRGB.
pub const TO_SRGB_WGSL: &str = concat!(
    include_str!(concat!(env!("OUT_DIR"), "/space.wgsl")),
    include_str!("space/to_srgb.wgsl")
);

/// The `from_srgb` conversion stage: [`WGSL`] and an `apply` that converts
/// premultiplied sRGB back to premultiplied working-space colour.
pub const FROM_SRGB_WGSL: &str = concat!(
    include_str!(concat!(env!("OUT_DIR"), "/space.wgsl")),
    include_str!("space/from_srgb.wgsl")
);

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
    pub const LINEAR_DISPLAY_P3: Self = Self { luma: P3_LUMA };
}

/// The `to_srgb` conversion stage on the CPU: premultiplied working-space
/// pixels to premultiplied sRGB, the transfer mirrored through zero so
/// extended values stay extended.
pub fn to_srgb(pixels: &mut [[f32; 4]]) {
    convert_space(pixels, |straight| {
        transform(&P3_TO_SRGB, straight).map(srgb_encode)
    });
}

/// The `from_srgb` conversion stage on the CPU: premultiplied sRGB back to
/// premultiplied working-space pixels.
pub fn from_srgb(pixels: &mut [[f32; 4]]) {
    convert_space(pixels, |straight| {
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
    let curve = if magnitude > SRGB_LINEAR_BREAK {
        SRGB_SCALE * powf(magnitude, SRGB_INVERSE_GAMMA) - SRGB_OFFSET
    } else {
        magnitude * SRGB_SLOPE
    };
    curve.copysign(linear)
}

/// The inverse of [`srgb_encode`]: one encoded channel back to linear,
/// mirrored through zero.
#[must_use]
pub fn srgb_decode(encoded: f32) -> f32 {
    let magnitude = encoded.abs();
    let curve = if magnitude > SRGB_ENCODED_BREAK {
        powf((magnitude + SRGB_OFFSET) / SRGB_SCALE, SRGB_GAMMA)
    } else {
        magnitude / SRGB_SLOPE
    };
    curve.copysign(encoded)
}

/// Applies `map` to each pixel's straight-alpha colour.
///
/// The colour is unpremultiplied by `max(alpha, 1e-6)` and premultiplied
/// again by alpha: the unpremultiply of the colour stages that floor alpha
/// (`LumaCurve`).
pub fn convert(pixels: &mut [[f32; 4]], map: impl Fn([f32; 3]) -> [f32; 3]) {
    for pixel in pixels {
        let alpha = pixel[3];
        let divisor = alpha.max(1.0e-6);
        let [r, g, b] = map([pixel[0] / divisor, pixel[1] / divisor, pixel[2] / divisor]);
        *pixel = [r * alpha, g * alpha, b * alpha, alpha];
    }
}

/// Moves each pixel between the working space and sRGB, as the conversion
/// stages (`space_to_srgb`, `space_from_srgb` in [`WGSL`]) do: the colour
/// is unpremultiplied by alpha, mapped, and premultiplied again. A pixel
/// with zero alpha carries no colour and becomes transparent black.
fn convert_space(pixels: &mut [[f32; 4]], map: impl Fn([f32; 3]) -> [f32; 3]) {
    for pixel in pixels {
        let alpha = pixel[3];
        if alpha == 0.0 {
            *pixel = [0.0; 4];
            continue;
        }
        let [r, g, b] = map([pixel[0] / alpha, pixel[1] / alpha, pixel[2] / alpha]);
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
