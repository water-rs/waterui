//! Crystallize filter implementation.

use crate::FilterDerive;

/// Creates a cell-like mosaic by snapping to jittered region centers.
#[derive(Debug, Clone, Copy, FilterDerive)]
#[filter(spatial, shader = "stylize/tiling/crystallize.wgsl")]
pub struct Crystallize<T>(pub T);

