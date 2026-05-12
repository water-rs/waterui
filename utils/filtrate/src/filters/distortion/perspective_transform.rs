//! Perspective transform filter implementation.

use crate::FilterDerive;

/// Maps a source quadrilateral into the output rectangle.
#[derive(Debug, Clone, FilterDerive)]
#[filter(spatial, shader = "distortion/perspective_transform.wgsl")]
pub struct PerspectiveTransform<T>(pub [T; 8]);

