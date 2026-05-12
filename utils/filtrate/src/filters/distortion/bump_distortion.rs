//! Bump distortion filter implementation.

use crate::FilterDerive;

/// Applies convex/concave bump distortion around a center.
#[derive(Debug, Clone, FilterDerive)]
#[filter(spatial, shader = "distortion/bump_distortion.wgsl")]
pub struct BumpDistortion<T>(pub [T; 4]);

