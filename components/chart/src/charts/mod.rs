//! High-level chart view wrappers.
//!
//! Each chart type wraps a `GpuSurface` with the appropriate renderer
//! and handles reactive data updates with animation.

pub mod area;
pub mod bar;
pub mod bubble;
pub mod candlestick;
pub mod choropleth;
pub mod contour;
pub mod depth;
pub mod gauge;
pub mod heatmap;
pub mod line;
pub mod pie;
pub mod radar;
pub mod scatter;
pub mod reactive;

pub use reactive::SignalRenderer;
