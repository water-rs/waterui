//! Core traits for GPU filter pipelines with automatic shader fusion.
//!
//! `filtrate-core` provides the foundational abstractions for building
//! lazy, zero-copy, fuseable GPU filter pipelines using wgpu.
//!
//! # Key Features
//!
//! - **Pure Data Filters**: Filters hold no GPU state - they're just recipes
//! - **Automatic Fusion**: Consecutive color-only filters fuse into single GPU pass
//! - **Zero-Allocation Params**: Nested tuples avoid heap allocation for parameters
//! - **Animation Support**: Uses nami's `Signal` trait for reactive filter parameters
//!
//! # Example
//!
//! ```ignore
//! use filtrate_core::{Filter, FilterExt};
//! use filtrate_core::filters::{Grayscale, Invert, Blur, Brightness};
//!
//! // Create a filter chain - type encodes fusion potential
//! let chain = Grayscale(1.0)
//!     .then(Invert)           // Fuses with Grayscale (both color-only)
//!     .then(Blur(5.0))        // Separate pass (spatial filter)
//!     .then(Brightness(0.2)); // After spatial, starts new potential fusion group
//!
//! // Type: Chain<Chain<Chain<Grayscale, Invert>, Blur>, Brightness>
//! // The pipeline compiler analyzes this to create optimal GPU passes
//! ```
//!
//! # Shader Fusion
//!
//! Filters with `COLOR_ONLY = true` (per-pixel operations) can be combined
//! into a single GPU pass. The [`Chain`] type preserves this information:
//!
//! ```ignore
//! // These all fuse: Grayscale → Invert → Brightness → Contrast
//! type AllColorOnly = Chain<Chain<Chain<Grayscale, Invert>, Brightness>, Contrast>;
//! assert!(AllColorOnly::COLOR_ONLY); // true - can be one pass
//!
//! // Blur breaks fusion: everything after needs separate handling
//! type WithBlur = Chain<AllColorOnly, Blur>;
//! assert!(!WithBlur::COLOR_ONLY); // false - needs multiple passes
//! ```

mod filter;
mod fragments;
mod params;

pub mod filters;

pub use filter::{Chain, Filter, FilterExt};
pub use fragments::FragmentList;
pub use params::ParamArray;

// Re-export dependencies for convenience
pub use nami;
pub use wgpu;
