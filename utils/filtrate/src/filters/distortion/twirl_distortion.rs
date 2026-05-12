//! Twirl distortion filter implementation.

use crate::Filter;

/// Applies a twirl distortion around a center point.
#[derive(Debug, Clone, Filter)]
#[filter(spatial, shader = "distortion/twirl_distortion.wgsl")]
pub struct TwirlDistortion<T>(pub [T; 4]);
