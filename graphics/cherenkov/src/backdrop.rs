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

pub use cherenkov_record::{BackdropEffect, BackdropShaderEffect, ColorMatrix, Refraction, Rim};

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
