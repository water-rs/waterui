//! Edge work filter implementation.

use crate::FilterDerive;

/// Highlights local edges using a Sobel-style gradient magnitude.
#[derive(Debug, Clone, FilterDerive)]
#[filter(spatial, shader = "edge_work.wgsl")]
pub struct EdgeWork<T>(pub [T; 2]);

