//! Edge work filter implementation.

use crate::Filter;

/// Highlights local edges using a Sobel-style gradient magnitude.
#[derive(Debug, Clone, Filter)]
#[filter(spatial, shader = "image/convolution/edge_work.wgsl")]
pub struct EdgeWork<T>(pub [T; 2]);
