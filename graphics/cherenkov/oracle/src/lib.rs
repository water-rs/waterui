//! Reference rasterizer and error metrics for the Cherenkov test suite.
//!
//! The oracle is deterministic, `f64`, single-threaded and favours clarity
//! over speed:
//!
//! - [`coverage`] computes the *exact area* of each pixel covered by a shape
//!   (curves flattened to `1e-4` px), for `NonZero` and `EvenOdd` rules;
//!   strokes are expanded with `kurbo`'s stroker.
//! - [`clip`] implements clipping as the **geometric intersection** of the
//!   shape edges with the clip edges — the exact area of
//!   `shape ∩ clip` — never a product of independently computed coverages.
//!   Engines that multiply coverages will show that as measured error.
//! - [`paint`] evaluates gradients and image patterns **at the pixel
//!   centre** and multiplies by exact coverage — the reference shading
//!   rule. Glyph outlines ([`glyphs`]) come from `skrifa`, unhinted.
//! - [`blend`] implements the 16 W3C Compositing and Blending Level 1
//!   modes in premultiplied linear Display P3.
//! - [`filter`] applies layer filters to a layer's isolated content.
//! - [`shadow`] convolves exact coverage with a true Gaussian in `f64`.
//! - [`metrics`] implements FLIP (and HDR-FLIP for values > `1.0`), ported
//!   from `FLIP.h` (NVlabs/flip, BSD-3-Clause — Andersson et al., HPG 2020
//!   and HDR-FLIP 2021), plus maximum local error (max per-channel
//!   absolute difference after a 3×3 box filter) and a magma error heatmap.

pub mod blend;
pub mod clip;
pub mod color;
pub mod coverage;
pub mod filter;
pub mod gamut;
pub mod glyphs;
pub mod image;
mod mesh;
pub mod metrics;
pub mod paint;
pub mod path;
pub mod present;
pub mod projective;
pub mod render;
pub mod resources;
mod sdf;
pub mod shadow;
pub mod tone;

pub use image::{F32Image, Image};
pub use metrics::Metrics;
pub use render::{RenderError, Renderer};
pub use resources::Resources;
