//! Typed image storage formats and the data a commit hands to the render
//! thread for [`Image`](crate::Image) registration.
//!
//! The image-data vocabulary is defined once in `cherenkov-record` — the
//! same types `waterui-graphics` registers — and re-exported here so the
//! engine's `crate::image` paths keep reading the same names.

pub use cherenkov_record::{
    Astc4x4, Bc7, Etc2Rgba, Format, ImageColorSpace, ImageData, ImageFormat, ImageUpload, Rgba8,
    Rgba16F,
};

/// The sRGB EOTF (encoded -> linear), odd through the origin.
///
/// `f(-x) = -f(x)` (CSS Color 4, IEC 61966-2-2 nonlinear): negative
/// (out-of-gamut) and above-one (HDR) components are transferred, never
/// clamped. This is the same definition as the oracle's
/// `cherenkov_oracle::color::srgb_decode` — the backends cannot depend on
/// the oracle, so the decode they share lives here.
#[must_use]
pub fn srgb_decode(c: f64) -> f64 {
    let a = c.abs();
    let e = if a <= 0.04045 {
        a / 12.92
    } else {
        ((a + 0.055) / 1.055).powf(2.4)
    };
    e.copysign(c)
}
