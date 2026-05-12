//! Line halftone filter implementation.

use crate::FilterDerive;

/// Renders luma as an angled line-screen halftone pattern.
#[derive(Debug, Clone, FilterDerive)]
#[filter(spatial, shader = "stylize/halftone/line_halftone.wgsl")]
pub struct LineHalftone<T>(pub [T; 4]);

