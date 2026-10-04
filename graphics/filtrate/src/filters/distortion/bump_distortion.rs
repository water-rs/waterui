//! Bump distortion filter implementation.

use crate::Filter;

/// Applies convex/concave bump distortion around a center.
///
/// Parameters: center x and y (uv), radius (1.0 = the shorter edge) and
/// scale. The displacement is bounded by the radius in isotropic units for
/// `scale >= -1`; below that it diverges and spans the image extent.
#[derive(Debug, Clone, Filter)]
#[filter(
    spatial,
    shader = "distortion/bump_distortion.wgsl",
    footprint_fn = crate::filters::footprint::bump
)]
pub struct BumpDistortion<T>(pub [T; 4]);
