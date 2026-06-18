#![cfg_attr(
    test,
    allow(clippy::float_cmp, reason = "tests assert exact filter parameter values")
)]
//! GPU filter library built on top of `filtrate-core` abstractions.
//!
//! `filtrate` hosts the built-in filter implementations and their WGSL
//! shaders, and (in upcoming phases) the GPU runtime that compiles a
//! [`Filter`] graph into a wgpu pipeline. It is designed to be usable
//! outside of `WaterUI` for any wgpu-based image, video, or render-target
//! workflow.
//!
//! # Layout
//!
//! - [`filters`]: built-in filter structs (`Brightness`, `Blur`, ...).
//!   Each filter implements [`Filter`] from `filtrate-core` and references
//!   one or more WGSL shader files under `src/shaders/`.
//! - `shaders/` (not a Rust module): WGSL fragments and full-shader files
//!   compiled into the binary via `include_str!` from individual filter
//!   modules.
//!
//! # Example
//!
//! ```ignore
//! use filtrate::{Filter, FilterExt};
//! use filtrate::filters::{Blur, Brightness};
//!
//! let chain = Blur(5.0).then(Brightness(0.1));
//! ```

pub mod effect;
pub mod filters;
pub mod multi_input;

pub use effect::{
    Effect, EffectContext, EffectInput, EffectOutput, EffectRenderResult, EffectSetupFuture,
    EffectSetupResult, ErasedEffect,
};
pub use filtrate_core::{
    AnimatedCallback, AnimatedTarget, AnimationTrack, Chain, Filter, FilterExt, FilterParam,
    FragmentList, Interpolator, MAX_FILTER_PARAM_VEC4S, MAX_FILTER_PARAMS, ParamArray,
    SignalVisitor, StageCollector, WatchGuard,
};

/// Procedural derive that generates a [`Filter`] implementation for a tuple
/// struct. See `filtrate-derive` for the supported `#[filter(...)]` shapes.
pub use filtrate_derive::Filter;
