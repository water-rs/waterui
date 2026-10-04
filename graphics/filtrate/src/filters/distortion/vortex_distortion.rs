//! Vortex distortion filter implementation.

use crate::Filter;

/// Applies a vortex-style spiral distortion.
///
/// Parameters: center x and y (uv), radius (1.0 = the shorter edge) and
/// angle (degrees). The displacement is bounded by the radius in isotropic
/// units, so the footprint is relative to the image extent.
#[derive(Debug, Clone, Filter)]
#[filter(
    spatial,
    shader = "distortion/vortex_distortion.wgsl",
    footprint_fn = crate::filters::footprint::radial
)]
pub struct VortexDistortion<T>(pub [T; 4]);
