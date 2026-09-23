//! Filter implementations.
//!
//! Each filter is a pure data struct implementing the [`Filter`](crate::Filter) trait.
//! Filters with `COLOR_ONLY = true` can be automatically fused with adjacent
//! color-only filters for better performance.

mod color;
mod distortion;
mod image;
mod stylize;

pub use color::*;
pub use distortion::*;
pub use image::*;
pub use stylize::*;
