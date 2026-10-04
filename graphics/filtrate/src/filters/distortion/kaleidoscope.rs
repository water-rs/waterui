//! Kaleidoscope filter implementation.

use crate::Filter;

/// Reflects content around repeated angular wedges.
///
/// Parameters: segments, rotation (degrees) and center x and y (uv). Any
/// pixel can read any other, so the footprint spans the whole image extent.
#[derive(Debug, Clone, Filter)]
#[filter(
    spatial,
    shader = "distortion/kaleidoscope.wgsl",
    footprint_extent = 1.0
)]
pub struct Kaleidoscope<T>(pub [T; 4]);
