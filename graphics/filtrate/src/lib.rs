// The executor's async setup nests deeply enough that clippy's `Send`
// analysis overflows the default limit and abandons the check — which then
// leaves the `future_not_send` expectation on it unfulfilled. Raise it so
// the lint actually runs.
#![recursion_limit = "256"]
#![cfg_attr(
    test,
    allow(
        clippy::float_cmp,
        reason = "tests assert exact filter parameter values"
    )
)]
//! A filter library: built-in filters as shader functions, and a reference
//! executor that runs them on wgpu textures.
//!
//! A [`Filter`] is pure data: the stages that apply it — each a WGSL function
//! for the shared shader composer, `cherenkov-shader` — and the parameters
//! that feed them. Filters are [`ColorFilter`]s or [`SpatialFilter`]s, and a
//! chain's kind is decided by the type system. The same definitions run in
//! the Cherenkov engine and in this crate's `Executor`, which composes a
//! chain and runs it pass by pass on any wgpu texture: images, decoded video
//! frames, or render targets.
//!
//! # Layout
//!
//! - [`filters`]: the built-in filters (`Brightness`, `Blur`, …).
//! - `Executor` (`gpu` feature): the reference wgpu executor, an `Effect`.
//! - `ShaderEffect` (`gpu` feature): an application-supplied WGSL fragment
//!   shader run over the input, an `Effect`.
//! - `shaders/` (not a Rust module): the stages' WGSL snippets, included by
//!   the filters that use them.
//!
//! # The `gpu` feature
//!
//! On by default, `gpu` carries everything that touches a GPU: the
//! `Executor`, the `Effect` runtime trait, `ShaderEffect`, and
//! `TextureImage` / texture-backed `FilterImage`s. With
//! `default-features = false` the crate has no wgpu in its dependency graph;
//! the filter definitions, their stage sources and the CPU kernels stay
//! available for consumers that only name and describe filters.
//!
//! # Example
//!
//! ```rust
//! use filtrate::filters::{Blur, Brightness};
//! #[cfg(feature = "gpu")]
//! use filtrate::Executor;
//! use filtrate::{Filter, FilterExt, Footprint, SpatialFilter};
//!
//! let chain = Blur(5.0_f32).then(Brightness(0.1_f32));
//! // A chain's params nest, one array per link, in application order.
//! assert_eq!(chain.params(), ([5.0], [0.1]));
//! // A blur makes the chain spatial; it reads five pixels each way.
//! assert_eq!(chain.footprint(), Footprint::pixels(5.0));
//! #[cfg(feature = "gpu")]
//! # {
//! let executor = Executor::new(chain);
//! # drop(executor);
//! # }
//! ```
//!
//! # WebGL support
//!
//! The optional `webgl` feature enables wgpu's WebGL2 backend for wasm
//! targets (`wasm32-unknown-unknown`); it implies `gpu`. Every pass is a
//! fragment pass, so the executor needs no compute shaders or storage
//! textures. The feature is a compile error on non-wasm targets.

// The `webgl` feature only makes sense where a WebGL2 context exists. On any
// other target it is inert (wgpu gates its WebGL backend to wasm), so turning
// it on natively is a build-graph mistake — fail fast instead of silently
// shipping an unused dependency set.
#[cfg(all(feature = "webgl", not(target_family = "wasm")))]
compile_error!(
    "filtrate's `webgl` feature is only supported on wasm targets \
     (wasm32-unknown-unknown); remove it from this build"
);

mod aux_image;
mod cpu;
#[cfg(feature = "gpu")]
pub mod effect;
#[cfg(feature = "gpu")]
mod executor;
pub mod filters;
#[cfg(feature = "gpu")]
pub mod shader_effect;

#[cfg(feature = "gpu")]
pub use aux_image::TextureImage;
pub use aux_image::{FilterImage, LutImage};
#[cfg(feature = "gpu")]
pub use effect::{
    Effect, EffectContext, EffectFrameClock, EffectFrameTiming, EffectInput, EffectOutput,
    EffectRedrawCallback, EffectRenderError, EffectRenderResult, EffectSetupError,
    EffectSetupResult, ShapeTextures,
};
#[cfg(feature = "gpu")]
pub use executor::Executor;
pub use filtrate_core::{
    AnimatedCallback, AnimatedTarget, AnimationTrack, AuxData, AuxFormat, AuxImage, AuxSource,
    Chain, ColorFilter, ColorStage, CpuFilter, CpuFilterError, CpuImage, CpuKernel, Filter,
    FilterExt, FilterParam, Footprint, ImageVisitor, Interpolator, OperatingSpace, ParamArray,
    ParamSource, Placed, ShapeInput, SignalVisitor, SpatialFilter, SpatialStage, StageCollector,
    WatchGuard, WorkingSpace, kind,
};

#[cfg(feature = "gpu")]
pub use shader_effect::{SHADER_EFFECT_MAX_PARAMS, ShaderEffect, ShaderEffectError};

/// Procedural derive that generates a single-stage filter: the [`Filter`]
/// implementation and its kind trait. See `filtrate-derive` for the
/// supported `#[filter(...)]` shapes.
pub use filtrate_derive::Filter;
