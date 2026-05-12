//! Pinch distortion filter implementation.

use crate::Filter;

/// Pinches or bulges content radially around a center.
#[derive(Debug, Clone, Filter)]
#[filter(spatial, shader = "distortion/pinch_distortion.wgsl")]
pub struct PinchDistortion<T>(pub [T; 4]);
