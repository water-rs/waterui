//! The built-in filters.
//!
//! Each filter is a pure data struct implementing [`Filter`](crate::Filter)
//! and exactly one of [`ColorFilter`](crate::ColorFilter) and
//! [`SpatialFilter`](crate::SpatialFilter). Its stages are WGSL snippets for
//! the shared composer, under this crate's `src/shaders/`.
//!
//! Colour filters that are linear maps on premultiplied RGBA say so through
//! [`ColorFilter::LINEAR`](crate::ColorFilter::LINEAR); spatial filters report
//! how far they read through
//! [`SpatialFilter::footprint`](crate::SpatialFilter::footprint). Every
//! filter operates in the linear working space and reads luma coefficients
//! from the working-space constants.
//!
//! # Declaring one
//!
//! Single-stage filters are written with [`#[derive(Filter)]`](crate::Filter):
//! one attribute names the kind, the WGSL snippet (resolved against the
//! declaring crate's `src/shaders/`), and the kind's properties, and the
//! fields flatten into the parameter array in declaration order.
//!
//! ```rust
//! use filtrate::{ColorFilter, Filter, SpatialFilter};
//!
//! #[derive(Filter)]
//! #[filter(color, shader = "color/adjustment/brightness.wgsl", linear = true)]
//! struct Brighten<T>(T);
//!
//! #[derive(Filter)]
//! #[filter(
//!     spatial,
//!     shader = "image/convolution/gradient.wgsl",
//!     footprint = 1.0,
//!     constants = [1.0, 1.0, 2.0]
//! )]
//! struct Edges;
//!
//! const { assert!(<Brighten<f32> as ColorFilter>::LINEAR) };
//! assert_eq!(Edges.footprint(), filtrate::Footprint::pixels(1.0));
//! assert_eq!(Brighten(0.2_f32).params(), [0.2]);
//! ```

mod color;
mod composite;
mod distortion;
mod footprint;
mod image;
mod stylize;

pub use color::*;
pub use composite::*;
pub use distortion::*;
pub use image::*;
pub use stylize::*;
