//! Gloom filter implementation.

use crate::FilterDerive;

/// Adds a dark halo around high-luminance regions.
#[derive(Debug, Clone, FilterDerive)]
#[filter(spatial, shader = "gloom.wgsl")]
pub struct Gloom<T>(pub [T; 3]);

