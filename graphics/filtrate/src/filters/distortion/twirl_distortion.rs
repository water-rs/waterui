//! Twirl distortion filter implementation.

use crate::Filter;

/// Applies a twirl distortion around a center point.
///
/// Parameters: center x and y (uv), radius (1.0 = the shorter edge) and
/// angle (degrees). The displacement is bounded by the radius in isotropic
/// units, so the footprint is relative to the image extent.
#[derive(Debug, Clone, Filter)]
#[filter(
    spatial,
    shader = "distortion/twirl_distortion.wgsl",
    footprint_fn = crate::filters::footprint::radial
)]
pub struct TwirlDistortion<T>(pub [T; 4]);
