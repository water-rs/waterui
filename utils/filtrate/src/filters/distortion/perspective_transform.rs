//! Perspective transform filter implementation.

use crate::Filter;

/// Maps a source quadrilateral into the output rectangle.
#[derive(Debug, Clone, Filter)]
#[filter(spatial, shader = "distortion/perspective_transform.wgsl")]
pub struct PerspectiveTransform<T>(pub [T; 8]);
