//! Programmable content effects attached to layers: shaders that sample a
//! layer's own isolated content against its live silhouette signed distance
//! and normal.
//!
//! Unlike [`BackdropEffect`](crate::BackdropEffect), which samples what lies
//! behind a layer, a content effect isolates the layer's own subtree
//! (content and children) and composites the shaded result onto the parent.

use std::borrow::Cow;

use crate::animation::AnimLanes;
use crate::backdrop::RecordingScale;
use crate::resource::ContentShaderId;

/// A registered content effect shader with its uniforms, in declared order.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ContentShaderEffect {
    /// The shader, as registered with the consumer.
    pub shader: ContentShaderId,
    /// Uniform values, packed four per `vec4` and zero-filled; at most 64.
    pub uniforms: Vec<f32>,
    /// The registered source's sampling reach in device pixels.
    #[cfg_attr(feature = "serde", serde(skip))]
    reach: f32,
    /// The device pixels per logical pixel of the layer's recording.
    #[cfg_attr(feature = "serde", serde(skip))]
    recording_scale: RecordingScale,
}

impl ContentShaderEffect {
    /// A content shader effect: `shader` is the registered shader's id,
    /// `uniforms` its uniform values in declared order, and `reach` the
    /// registered source's sampling reach in device pixels.
    #[must_use]
    pub const fn new(shader: ContentShaderId, uniforms: Vec<f32>, reach: f32) -> Self {
        Self {
            shader,
            uniforms,
            reach,
            recording_scale: RecordingScale::ONE,
        }
    }

    /// Sets the device pixels one logical pixel of the layer's recording
    /// spans, handed to the shader as `px.scale` ([`ContentShaderSource`]).
    #[must_use]
    pub fn scale(self, scale: RecordingScale) -> Self {
        Self {
            recording_scale: scale,
            ..self
        }
    }

    /// The effect's sampling reach in device pixels.
    #[must_use]
    pub const fn reach(&self) -> f32 {
        self.reach
    }

    /// The layer's recording scale ([`RecordingScale::ONE`] unless
    /// [`Self::scale`] set it).
    #[must_use]
    pub const fn recording_scale(&self) -> RecordingScale {
        self.recording_scale
    }
}

/// A content effect animates its uniforms lane by lane when both endpoints
/// name the same shader, recording scale and uniform count. Any other pair
/// snaps.
impl AnimLanes for ContentShaderEffect {
    fn anim_lanes(&self, target: &Self) -> Option<Box<[f64]>> {
        if self.shader != target.shader
            || self.recording_scale != target.recording_scale
            || self.uniforms.len() != target.uniforms.len()
        {
            return None;
        }
        Some(
            self.uniforms
                .iter()
                .copied()
                .map(f64::from)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        )
    }

    fn with_lanes(&self, lanes: &[f64]) -> Self {
        Self {
            shader: self.shader,
            uniforms: lanes.iter().copied().map(f64_as_f32).collect(),
            reach: self.reach,
            recording_scale: self.recording_scale,
        }
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "a lane interpolates between two f32 parameters, so it fits an f32"
)]
const fn f64_as_f32(value: f64) -> f32 {
    value as f32
}

nami_core::impl_constant!(ContentShaderEffect);

/// A content effect shader's source and sampling reach.
///
/// The source defines
///
/// ```wgsl
/// struct ContentPixel {
///     p: vec2<f32>, sdf: f32, normal: vec2<f32>,
///     size: vec2<f32>, scale: f32, local: vec2<f32>,
/// }
/// fn content_effect(px: ContentPixel, params: array<vec4<f32>, 16>) -> vec4<f32>
/// ```
///
/// where `px.p` is the pixel centre in device space, `px.sdf` the signed
/// distance to the layer's own clip silhouette (negative inside),
/// `px.normal` that silhouette's unit outward normal, `px.size` the
/// silhouette's device bounds size, `px.scale` the device pixels per
/// logical pixel of the layer's recording ([`ContentShaderEffect::scale`]),
/// `px.local` the pixel in the layer's own coordinate space, and `params`
/// the effect uniforms packed four per `vec4`, zero-filled.
///
/// `fn content_sample(q: vec2<f32>) -> vec4<f32>` bilinearly samples the
/// layer's isolated content at device point `q`, returning premultiplied
/// extended linear Display P3 and transparent outside the isolated content
/// bounds. The return value of `content_effect` is premultiplied extended
/// linear Display P3 and written unclamped.
#[derive(Clone, Debug)]
pub struct ContentShaderSource {
    /// The fragment source, without the backend's prelude.
    pub source: Cow<'static, str>,
    /// The maximum displacement `content_sample` coordinates can take, in
    /// device pixels; non-negative and finite. Banded executors pad their
    /// isolation window by this much so displaced samples stay inside it.
    pub reach: f32,
}

impl ContentShaderSource {
    /// A content effect shader from WGSL.
    pub fn wgsl(source: impl Into<Cow<'static, str>>) -> Self {
        Self {
            source: source.into(),
            reach: 0.0,
        }
    }

    /// Sets the reach (see [`ContentShaderSource::reach`]).
    #[must_use]
    pub fn reach(self, px: f32) -> Self {
        Self {
            source: self.source,
            reach: px,
        }
    }
}
