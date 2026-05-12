//! Dot halftone filter implementation.

use crate::Filter;

/// Renders luma as a dot-screen halftone pattern.
#[derive(Debug, Clone, Filter)]
#[filter(spatial, shader = "stylize/halftone/dot_halftone.wgsl")]
pub struct DotHalftone<T>(pub [T; 4]);
