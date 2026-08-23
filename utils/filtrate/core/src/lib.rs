#![no_std]
#![cfg_attr(
    test,
    allow(
        clippy::float_cmp,
        reason = "tests assert exact filter parameter values"
    )
)]
//! Long-term stable abstractions for GPU filter pipelines.
//!
//! `filtrate-core` provides the foundational trait surface for declaring GPU
//! filters as pure data: a [`Filter`] knows its shader fragments, parameter
//! layout, and whether it can be fused with adjacent color-only filters. The
//! actual GPU runtime lives in the `filtrate` crate; built-in filter
//! implementations and their WGSL shaders live there as well.
//!
//! This crate aims for a stable 1.0 surface and intentionally has no `wgpu`
//! or reactive-system (e.g. `nami`) dependencies. Reactive frontends
//! provide their own [`FilterParam`] implementations on top of these
//! abstractions.
//!
//! # Key abstractions
//!
//! - [`Filter`]: pure-data description of a GPU filter recipe.
//! - [`Chain<A, B>`]: type-level composition that preserves fusion potential.
//! - [`FilterExt::then`]: ergonomic chain construction.
//! - [`ParamArray`]: zero-allocation parameter layout for nested tuples and
//!   fixed-size arrays.
//! - [`FilterParam`] / [`Interpolator`]: reactive-system-agnostic parameter
//!   abstraction with mandatory change observation.
//! - [`StageCollector`] / [`SignalVisitor`]: visitors used by the runtime
//!   to walk a filter's GPU stages and reactive parameters.
//!
//! # Example
//!
//! A filter is pure data: a parameter layout plus the GPU stages those
//! parameters feed. The built-in filters in `filtrate` are written exactly
//! this way (through `#[derive(Filter)]`), and a hand-written one composes
//! with them:
//!
//! ```rust
//! use filtrate_core::{Chain, Filter, FilterExt, StageCollector};
//!
//! struct Brightness(f32);
//!
//! impl Filter for Brightness {
//!     const COLOR_ONLY: bool = true;
//!     type Params = [f32; 1];
//!
//!     fn params(&self) -> [f32; 1] {
//!         [self.0]
//!     }
//!
//!     fn collect_stages<C: StageCollector>(&self, collector: &mut C) {
//!         // Real filters hand over a WGSL file through `include_str!`.
//!         collector.color_fragment("// WGSL: rgb += params[0];", 1);
//!     }
//! }
//!
//! let chain = Brightness(0.2).then(Brightness(-0.1));
//!
//! // Both links are color-only, so the chain stays fusable into one pass,
//! // and its parameters nest one array per link.
//! assert!(<Chain<Brightness, Brightness>>::COLOR_ONLY);
//! assert_eq!(chain.params(), ([0.2], [-0.1]));
//! ```

mod animation;
mod filter;
mod param;
mod params;
mod stage;
mod visitor;

pub use animation::AnimationTrack;
pub use filter::{Chain, Filter, FilterExt};
pub use param::{AnimatedCallback, AnimatedTarget, FilterParam, Interpolator, WatchGuard};
pub use params::ParamArray;
pub use stage::StageCollector;
pub use visitor::SignalVisitor;

/// Maximum number of `f32` parameters a single fused filter pipeline can carry.
///
/// This budget is shared between Rust-side uniform packing and WGSL shaders;
/// the runtime emits an `array<vec4<f32>, MAX_FILTER_PARAM_VEC4S>` slot, so
/// chain length is bounded by [`MAX_FILTER_PARAMS`] across all fused filters.
pub const MAX_FILTER_PARAMS: usize = 64;

/// Number of `vec4<f32>` slots required to hold [`MAX_FILTER_PARAMS`] floats.
pub const MAX_FILTER_PARAM_VEC4S: usize = MAX_FILTER_PARAMS / 4;

const _: () = assert!(MAX_FILTER_PARAMS == MAX_FILTER_PARAM_VEC4S * 4);
