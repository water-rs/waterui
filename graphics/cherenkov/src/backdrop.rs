//! Backdrop sampling: the per-member effect types a layer edit carries
//! live in `cherenkov-record` (a layer edit names them without the
//! engine); the engine-side shader *source* — the WGSL the render loop
//! compiles — stays here.
//!
//! A [`BackdropSample`] names the group a layer samples; an optional
//! [`BackdropEffect`] is evaluated inside the member's composite against
//! the group's shared filtered capture. Effects work in the extended
//! linear Display P3 working space and are never clamped.

use std::borrow::Cow;

/// The resolution a backdrop group captures at, as a fraction `s` of
/// device resolution, `0 < s ≤ 1`; fixed when the group is created
/// ([`Surface::backdrop_group`](crate::Surface::backdrop_group)).
///
/// The capture grid is anchored at the device origin: capture texel
/// `(i, j)` covers the device rect `[i/s, (i+1)/s) × [j/s, (j+1)/s)` and
/// holds the area-weighted mean of the backdrop over that rect's part
/// inside the surface, so a region of `w × h` device pixels resolves into
/// about `⌈w·s⌉ × ⌈h·s⌉` texels. The group's filter chain runs on the
/// capture, its footprint counted in capture texels — a device-pixel apron
/// of footprint ÷ `s` — and members sample it bilinearly at `p · s` for a
/// device point `p`. [`CaptureScale::FULL`] is the 1:1 capture.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct CaptureScale(f32);

/// Why a [`CaptureScale`] could not be constructed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CaptureScaleError {
    /// The scale is NaN or infinite.
    #[error("the capture scale is not finite")]
    NonFinite,
    /// The scale is not in `(0, 1]`.
    #[error("the capture scale must be above 0 and at most 1")]
    OutOfRange,
}

impl CaptureScale {
    /// Device resolution: the capture copies the backdrop 1:1.
    pub const FULL: Self = Self(1.0);

    /// A capture at `scale` times device resolution.
    ///
    /// # Errors
    /// [`CaptureScaleError::NonFinite`] when `scale` is NaN or infinite,
    /// [`CaptureScaleError::OutOfRange`] unless `0 < scale ≤ 1`.
    pub fn new(scale: f32) -> Result<Self, CaptureScaleError> {
        if !scale.is_finite() {
            return Err(CaptureScaleError::NonFinite);
        }
        if scale <= 0.0 || scale > 1.0 {
            return Err(CaptureScaleError::OutOfRange);
        }
        Ok(Self(scale))
    }

    /// The scale `s`.
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }

    /// Whether this is [`CaptureScale::FULL`]: the capture is a 1:1 copy.
    #[must_use]
    pub fn is_full(self) -> bool {
        // The constructor bounds the scale above by 1.
        self.0 >= 1.0
    }
}

/// A WGSL shader compiled for the backdrop composite contract
/// ([`Engine::backdrop_shader`](crate::Engine::backdrop_shader)), not a
/// shader paint.
///
/// The source defines
///
/// ```wgsl
/// fn backdrop_effect(p: vec2<f32>, sdf: f32, normal: vec2<f32>,
///                    size: vec2<f32>, params: array<vec4<f32>, 16>) -> vec4<f32>
/// ```
///
/// where `p` is the member pixel centre in device space, `sdf` the signed
/// distance to the member's clip edge (negative inside), `normal` the
/// unit outward normal, `size` the member's device bounds size, and
/// `params` the effect uniforms packed four per `vec4`, zero-filled.
/// `fn backdrop_sample(q: vec2<f32>) -> vec4<f32>` bilinearly samples the
/// filtered capture at device point `q` — at `q · s` on a group's capture
/// grid ([`CaptureScale`]) — clamped to its region. The return value is
/// premultiplied and written unclamped.
#[derive(Clone, Debug)]
pub struct BackdropShaderSource {
    /// The fragment source, without the backend's prelude.
    pub source: Cow<'static, str>,
    /// The maximum displacement `backdrop_sample` coordinates can take,
    /// in device pixels; non-negative and finite. The group's capture
    /// region grows by this much around the member so displaced samples
    /// stay inside it.
    pub reach: f32,
}

impl BackdropShaderSource {
    /// A backdrop effect shader from WGSL.
    pub fn wgsl(source: impl Into<Cow<'static, str>>) -> Self {
        Self {
            source: source.into(),
            reach: 0.0,
        }
    }

    /// Sets the reach (see [`BackdropShaderSource::reach`]).
    #[must_use]
    pub fn reach(self, px: f32) -> Self {
        Self {
            source: self.source,
            reach: px,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CaptureScale, CaptureScaleError};

    #[test]
    fn capture_scale_is_finite_and_in_the_unit_interval() {
        for scale in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(CaptureScale::new(scale), Err(CaptureScaleError::NonFinite));
        }
        for scale in [0.0, -0.0, -0.25, 1.0 + f32::EPSILON, 2.0] {
            assert_eq!(CaptureScale::new(scale), Err(CaptureScaleError::OutOfRange));
        }
        assert_eq!(CaptureScale::new(1.0), Ok(CaptureScale::FULL));
        assert!(CaptureScale::FULL.is_full());
        let quarter = CaptureScale::new(0.25).expect("in range");
        assert!(!quarter.is_full());
        assert!((quarter.get() - 0.25).abs() <= f32::EPSILON);
    }
}
