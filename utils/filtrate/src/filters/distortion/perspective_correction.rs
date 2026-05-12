//! Perspective correction filter implementation.

use crate::Filter;

/// Corrects a perspective-skewed quadrilateral back to a rectangle.
#[derive(Debug, Clone, Filter)]
#[filter(spatial, shader = "distortion/perspective_correction.wgsl")]
pub struct PerspectiveCorrection<T>(pub [T; 8]);
