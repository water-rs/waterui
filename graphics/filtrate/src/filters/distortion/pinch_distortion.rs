//! Pinch distortion filter implementation.

use crate::Filter;

/// Pinches or bulges content radially around a center.
///
/// Parameters: center x and y (uv), radius (1.0 = the shorter edge) and
/// scale. The displacement is bounded by the radius in isotropic units for
/// `scale >= -1`; below that it diverges and spans the image extent.
#[derive(Debug, Clone, Filter)]
#[filter(
    spatial,
    shader = "distortion/pinch_distortion.wgsl",
    footprint_fn = crate::filters::footprint::pinch
)]
pub struct PinchDistortion<T>(pub [T; 4]);
