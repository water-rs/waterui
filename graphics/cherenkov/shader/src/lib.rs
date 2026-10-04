//! The shader composer shared by Cherenkov and filtrate.
//!
//! Every shader fragment is a naga function. A [`Snippet`] is authored as WGSL
//! source, parsed by naga's WGSL front end, and checked against a declared
//! ABI. [`compose`] builds one naga [`Module`](naga::Module) from a chain of
//! snippets by working on the IR: it imports each snippet's function (with
//! its types, constants and helpers), generates the functions that sequence
//! them, rewrites texture samples to fold a colour prefix into a spatial
//! stage, specializes parameters that are constant, compacts the module, and
//! validates the result. No shader text is ever built by concatenation.
//!
//! # Snippet ABI
//!
//! A snippet defines exactly one function named `apply`.
//!
//! A **colour** snippet maps a colour to a colour:
//!
//! ```wgsl
//! struct Params { amount: f32 }
//! fn apply(color: vec4<f32>, params: Params) -> vec4<f32> { /* … */ }
//! ```
//!
//! A **spatial** snippet samples its input around a coordinate:
//!
//! ```wgsl
//! struct Params { radius: f32 }
//! fn apply(input: texture_2d<f32>, input_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, params: Params) -> vec4<f32> { /* … */ }
//! ```
//!
//! `size` is the extent of `input` in pixels, supplied by the executor —
//! a spatial snippet must not call `textureDimensions` on `input` itself
//! (auxiliary images and `shape` keep their own queries).
//!
//! The sampler argument declares its filter mode by name: `input_sampler`
//! allows the executor to bind a filtering sampler, while
//! `input_point_sampler` requires a nearest (point) sampler — the contract a
//! stage needs for its samples to fold a colour prefix (see [`FoldBlocker`]).
//!
//! After the required arguments, a snippet may declare, in any order:
//!
//! - `params: Params`: a struct whose members are `f32`, `vec2<f32>`,
//!   `vec3<f32>` or `vec4<f32>`;
//! - `space: WorkingSpace`: the engine-provided working-space constants, a
//!   struct exactly equivalent to [`WORKING_SPACE_WGSL`] (same members,
//!   offsets and span; the name is free);
//! - spatial snippets only: `shape: texture_2d<f32>` (the clip shape's signed
//!   distance field or mask), and `aux0`, `aux1`, … `: texture_2d<f32>`
//!   (auxiliary images, numbered without gaps).
//!
//! Colours are premultiplied, in the linear working space. Parameters are
//! always `f32`, whatever precision the colour values use.
//!
//! # Variants
//!
//! The `f32` source is required. A snippet may also declare an `f16` variant,
//! in which the colour values (`color`, the result, and a spatial snippet's
//! result) are `vec4<f16>`, and subgroup variants that use subgroup
//! operations. [`compose`] uses a declared variant wherever it matches the
//! requested [`ComposeOptions`] and reports the variant each stage used.
//!
//! # Libraries
//!
//! A [`LibrarySource`] is a WGSL source of plain helper functions — not a
//! stage — registered with a [`SnippetSource`]. The snippet calls the
//! library's functions by name; the composer imports each referenced helper
//! exactly once per composed module, deduplicated by library identity, and
//! rejects name collisions between libraries and between a library and a
//! snippet. A snippet's `f16` variant requires the library's `f16` source.
//!
//! # What the composer decides, and what it leaves to the executor
//!
//! The composer makes no execution decision. It returns a normalized
//! [`Composition`]: a sequence of [`Piece`]s whose boundaries are the possible
//! materialization points. Where a colour piece precedes a spatial piece, it
//! also offers a folded alternative, with the cost parameters an executor
//! needs to choose ([`FoldCost`]). Executors (filtrate's reference executor,
//! or Cherenkov itself) wrap the segment functions into entry points and pick
//! among the alternatives.
//!
//! Composed modules call snippet functions rather than splicing their bodies
//! together. Every platform shader compiler inlines small functions and then
//! folds the specialized constants; naga-level inlining would add nothing
//! but work.
//!
//! An executor wraps a segment into an entry point on the IR too:
//! [`FunctionBuilder`] builds the entry point's function, with bound
//! arguments and a bound result, and [`validate`] checks the module it adds
//! it to.
//!
//! # Features
//!
//! - `eval`: the `eval` module, a reference interpreter that runs snippets and
//!   composed segments on the CPU, for tests and for cross-checking CPU
//!   implementations against their shaders.

mod abi;
mod builder;
mod chain;
mod emit;
mod errors;
#[cfg(feature = "eval")]
pub mod eval;
mod import;
mod parse;
mod rewrite;

pub use abi::{
    FoldBlocker, Param, ParamType, ParamValue, Precision, SamplerFilter, SnippetKind,
    WORKING_SPACE_WGSL,
};
pub use builder::FunctionBuilder;
pub use chain::{
    ComposeOptions, Composition, FoldCost, Folded, Piece, Segment, SegmentArg, Stage,
    UniformLayout, UniformMember, compose,
};
pub use emit::{msl, spirv, validate, wgsl};
pub use errors::{ComposeError, EmitError, SnippetError};
/// The naga version modules are built with; the same one wgpu 30 links.
pub use naga;
pub use parse::{LibrarySource, SampleCount, Snippet, SnippetSource, Variant};
