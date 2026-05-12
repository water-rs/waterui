//! Gloom filter implementation.

use crate::FilterDerive;

/// Adds a dark halo around high-luminance regions.
#[derive(Debug, Clone, FilterDerive)]
#[filter(spatial, shader = "stylize/lighting/gloom.wgsl")]
pub struct Gloom<T>(pub [T; 3]);

