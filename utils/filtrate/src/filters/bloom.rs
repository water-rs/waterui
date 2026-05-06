//! Bloom filter implementation.

use crate::FilterDerive;

/// Adds a bright glow around high-luminance regions.
#[derive(Debug, Clone, FilterDerive)]
#[filter(spatial, shader = "bloom.wgsl")]
pub struct Bloom<T>(pub [T; 3]);

