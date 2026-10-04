//! Per-member backdrop sampling effects.
//!
//! A [`BackdropSample`] names the group a layer samples; an optional
//! [`BackdropEffect`] is evaluated inside the member's composite against
//! the group's shared filtered capture. Effects work in the extended
//! linear Display P3 working space and are never clamped.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::message::BackdropShaderId;

/// A per-member effect evaluated in the member's composite against the
/// group's shared filtered capture. Extended linear P3, no clamping.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorMatrix(pub [f32; 12]);

/// Displaces the backdrop sample point along the member's edge normal.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Refraction {
    /// The rim depth over which the displacement decays, in device
    /// pixels; must be positive.
    pub depth: f32,
    /// The maximum displacement at the edge, in device pixels; must be
    /// non-negative.
    pub strength: f32,
}

/// A highlight inside the member clip's rim, additive on the sample.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BackdropShaderEffect {
    /// The shader, registered with
    /// [`Engine::backdrop_shader`](crate::Engine::backdrop_shader).
    pub shader: BackdropShaderId,
    /// Uniform values, packed four per `vec4` and zero-filled; at most 64.
    pub uniforms: Vec<f32>,
    /// The registered source's reach, copied by
    /// [`BackdropShader::effect`](crate::BackdropShader::effect).
    #[serde(skip)]
    pub(crate) reach: f32,
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
/// filtered capture, clamped to its region. The return value is
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
