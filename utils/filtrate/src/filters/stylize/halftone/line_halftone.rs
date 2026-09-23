//! Line halftone filter implementation.

use crate::Filter;

/// Renders luma as an angled line-screen halftone pattern.
#[derive(Debug, Clone, Filter)]
#[filter(spatial, shader = "stylize/halftone/line_halftone.wgsl")]
pub struct LineHalftone<T>(pub [T; 4]);
