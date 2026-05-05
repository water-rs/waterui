//! Long-term stable abstractions for GPU filter pipelines.
//!
//! `filtrate-core` provides the foundational trait surface for declaring GPU
//! filters as pure data: a [`Filter`] knows its shader fragments, parameter
//! layout, and whether it can be fused with adjacent color-only filters. The
//! actual GPU runtime lives in the `filtrate` crate; built-in filter
//! implementations and their WGSL shaders live there as well.
//!
//! This crate aims for a stable 1.0 surface and intentionally has no `wgpu`,
//! `nami`, or shader-string dependencies.
//!
//! # Key abstractions
//!
//! - [`Filter`]: pure-data description of a GPU filter recipe.
//! - [`Chain<A, B>`]: type-level composition that preserves fusion potential.
//! - [`FilterExt::then`]: ergonomic chain construction.
//! - [`ParamArray`]: zero-allocation parameter layout for nested tuples and
//!   fixed-size arrays.
//! - [`FragmentList`]: zero-allocation shader fragment composition.
//!
//! # Example
//!
//! ```ignore
//! use filtrate_core::{Filter, FilterExt};
//! use filtrate::filters::{Grayscale, Invert, Blur, Brightness};
//!
//! // Type encodes fusion potential at compile time.
//! let chain = Grayscale(1.0)
//!     .then(Invert)         // Fuses with Grayscale (both color-only)
//!     .then(Blur(5.0))      // Separate pass (spatial filter)
//!     .then(Brightness(0.2));
//! ```

mod filter;
mod fragments;
mod params;

pub use filter::{Chain, Filter, FilterExt};
pub use fragments::FragmentList;
pub use params::ParamArray;

/// Maximum number of `f32` parameters a single fused filter pipeline can carry.
///
/// This budget is shared between Rust-side uniform packing and WGSL shaders;
/// the runtime emits an `array<vec4<f32>, MAX_FILTER_PARAM_VEC4S>` slot, so
/// chain length is bounded by [`MAX_FILTER_PARAMS`] across all fused filters.
pub const MAX_FILTER_PARAMS: usize = 64;

/// Number of `vec4<f32>` slots required to hold [`MAX_FILTER_PARAMS`] floats.
pub const MAX_FILTER_PARAM_VEC4S: usize = MAX_FILTER_PARAMS / 4;

const _: () = assert!(MAX_FILTER_PARAMS == MAX_FILTER_PARAM_VEC4S * 4);
