//! Core traits for GPU filter pipelines.
//!
//! `filtrate-core` provides the foundational abstractions for building
//! lazy, zero-copy, fuseable GPU pipelines using wgpu.
//!
//! # Key Types
//!
//! - [`RenderContext`]: Shared context for recording commands without submission.
//! - [`Source`]: Trait for lazy texture sources that encode generation commands.
//! - [`Filter`]: Trait for filters that encode processing commands.
//! - [`Pipeline`]: Builder for chaining sources and filters with lazy execution.
//!
//! # Example
//!
//! ```ignore
//! use filtrate_core::Pipeline;
//! use filtrate_barcode::Barcode;
//! use filtrate::{Blur, Saturation};
//!
//! // Chaining builds a lazy Pipeline - nothing executes yet
//! let pipeline = Barcode::new("https://waterui.dev")
//!     .blur(5.0)
//!     .saturate(1.5);
//!
//! // render() triggers fused execution (single queue.submit)
//! pipeline.render(device, queue, &output);
//! ```

#![allow(clippy::multiple_crate_versions)]

mod context;
mod filter;
mod pipeline;
mod source;

pub use context::RenderContext;
pub use filter::Filter;
pub use pipeline::{Pipeline, SourceExt};
pub use source::Source;

// Re-export wgpu for convenience
pub use wgpu;
