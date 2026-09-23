//! Bump distortion filter implementation.

use crate::Filter;

/// Applies convex/concave bump distortion around a center.
#[derive(Debug, Clone, Filter)]
#[filter(spatial, shader = "distortion/bump_distortion.wgsl")]
pub struct BumpDistortion<T>(pub [T; 4]);
