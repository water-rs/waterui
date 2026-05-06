//! Dot halftone filter implementation.

use crate::FilterDerive;

/// Renders luma as a dot-screen halftone pattern.
#[derive(Debug, Clone, FilterDerive)]
#[filter(spatial, shader = "dot_halftone.wgsl")]
pub struct DotHalftone<T>(pub [T; 4]);

