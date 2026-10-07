//! Backdrop sampling attached to layers: a [`BackdropSample`] names the
//! group a layer samples.
//!
//! An optional [`BackdropEffect`] is evaluated inside the member's
//! composite against the group's shared filtered capture. Effects work in
//! the extended linear Display P3 working space and are never clamped.
//!
//! The engine-independent parameters of a group and of a custom effect —
//! the [`CaptureScale`] a group captures at and the
//! [`BackdropShaderSource`] a backdrop shader compiles from — live here
//! too, so a widget theme can declare them in a
//! [`MaterialRegistry`](crate::MaterialRegistry) without an engine.

use std::borrow::Cow;

use crate::BackdropShaderId;
use crate::ops::BackdropId;

/// A per-member effect evaluated in the member's composite against the
/// group's shared filtered capture. Extended linear P3, no clamping.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BackdropEffect {
    /// A 3×4 premultiplied RGBA matrix in filtrate's `ColorMatrix` layout:
    /// row-major, three rows of `[r, g, b, bias]` applied as
    /// `dot(row, c)` to the premultiplied sample (the bias column thus
    /// scales with alpha); alpha passes through.
    Color(ColorMatrix),
    /// Displaces the sample point along the member's inward edge normal:
    /// `q = p - n · strength · t²` with `t = clamp(1 + d/depth, 0, 1)`,
    /// `d` the signed device-space distance to the clip edge.
    Refraction(Refraction),
    /// A highlight inside the clip's rim: `c = sample(p)`,
    /// `t = clamp(1 + d / width, 0, 1)` and
    /// `c.rgb += color.rgb · color.a · gain · t²`, alpha unchanged.
    Rim(Rim),
    /// A registered backdrop shader with its uniforms (declared order).
    Shader(BackdropShaderEffect),
}

impl From<ColorMatrix> for BackdropEffect {
    fn from(matrix: ColorMatrix) -> Self {
        Self::Color(matrix)
    }
}

impl From<Refraction> for BackdropEffect {
    fn from(refraction: Refraction) -> Self {
        Self::Refraction(refraction)
    }
}

impl From<Rim> for BackdropEffect {
    fn from(rim: Rim) -> Self {
        Self::Rim(rim)
    }
}

impl From<BackdropShaderEffect> for BackdropEffect {
    fn from(shader: BackdropShaderEffect) -> Self {
        Self::Shader(shader)
    }
}

/// A 3×4 premultiplied colour matrix (filtrate `ColorMatrix` layout).
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ColorMatrix(pub [f32; 12]);

/// Displaces the backdrop sample point along the member's edge normal.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Refraction {
    /// The rim depth over which the displacement decays, in device
    /// pixels; must be positive.
    pub depth: f32,
    /// The maximum displacement at the edge, in device pixels; must be
    /// non-negative.
    pub strength: f32,
}

/// A highlight inside the member clip's rim, additive on the sample.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Rim {
    /// The rim's width inside the clip edge, in device pixels; must be
    /// positive.
    pub width: f32,
    /// The highlight colour, straight-alpha linear Display P3 (the
    /// working space); must be finite.
    pub color: [f32; 4],
    /// The highlight's gain, finite; values above 1 push the rim above
    /// SDR white.
    pub gain: f32,
}

/// A registered backdrop shader with its uniforms, in declared order.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BackdropShaderEffect {
    /// The shader, as registered with the consumer.
    pub shader: BackdropShaderId,
    /// Uniform values, packed four per `vec4` and zero-filled; at most 64.
    pub uniforms: Vec<f32>,
    /// The registered source's reach in device pixels.
    #[cfg_attr(feature = "serde", serde(skip))]
    reach: f32,
}

impl BackdropShaderEffect {
    /// A shader effect: `shader` is the registered shader's id, `uniforms`
    /// its uniform values in declared order, and `reach` the registered
    /// source's sampling reach in device pixels.
    #[must_use]
    pub const fn new(shader: BackdropShaderId, uniforms: Vec<f32>, reach: f32) -> Self {
        Self {
            shader,
            uniforms,
            reach,
        }
    }
}

impl BackdropEffect {
    /// The effect's sampling reach in device pixels: how far beyond the
    /// member's bounds its samples can land. `Color` and `Rim` read the
    /// member pixel only; `Refraction` reaches `strength`; a shader
    /// reaches its registered `reach`.
    #[must_use]
    pub const fn reach(&self) -> f32 {
        match self {
            Self::Color(_) | Self::Rim(_) => 0.0,
            Self::Refraction(r) => r.strength,
            Self::Shader(s) => s.reach,
        }
    }
}

/// A layer's backdrop sample: the group it samples and an optional
/// per-member effect evaluated in the member's composite against the
/// shared filtered capture.
#[derive(Clone, Debug, PartialEq)]
pub struct BackdropSample {
    /// The sampled group.
    group: BackdropId,
    /// The per-member effect applied in the member's composite.
    effect: Option<BackdropEffect>,
}

impl BackdropSample {
    /// A plain sample of `group`, with no per-member effect.
    #[must_use]
    pub const fn new(group: BackdropId) -> Self {
        Self {
            group,
            effect: None,
        }
    }

    /// A sample of `group` with a per-member `effect`, evaluated in the
    /// member's composite against the shared filtered capture.
    #[must_use]
    pub fn with_effect(group: BackdropId, effect: impl Into<BackdropEffect>) -> Self {
        Self {
            group,
            effect: Some(effect.into()),
        }
    }

    /// The sampled group.
    #[must_use]
    pub const fn group(&self) -> BackdropId {
        self.group
    }

    /// The per-member effect, when the sample was made with
    /// [`BackdropSample::with_effect`].
    #[must_use]
    pub const fn effect(&self) -> Option<&BackdropEffect> {
        self.effect.as_ref()
    }
}

nami_core::impl_constant!(BackdropSample);

/// The resolution a backdrop group captures at, as a fraction `s` of
/// device resolution, `0 < s ≤ 1`; fixed when the group is created
/// (the engine's `Surface::backdrop_group`).
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

/// A WGSL shader compiled for the backdrop composite contract (the
/// engine's `Engine::backdrop_shader`), not a shader paint.
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
