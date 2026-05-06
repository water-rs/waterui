//! Pinch distortion filter implementation.

use crate::FilterDerive;

/// Pinches or bulges content radially around a center.
#[derive(Debug, Clone, FilterDerive)]
#[filter(spatial, shader = "pinch_distortion.wgsl")]
pub struct PinchDistortion<T>(pub [T; 4]);

