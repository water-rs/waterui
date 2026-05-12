//! Perspective correction filter implementation.

use crate::FilterDerive;

/// Corrects a perspective-skewed quadrilateral back to a rectangle.
#[derive(Debug, Clone, FilterDerive)]
#[filter(spatial, shader = "distortion/perspective_correction.wgsl")]
pub struct PerspectiveCorrection<T>(pub [T; 8]);

