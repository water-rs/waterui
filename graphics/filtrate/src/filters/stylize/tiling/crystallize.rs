//! Crystallize filter implementation.

use crate::Filter;

/// Creates a cell-like mosaic by snapping to jittered region centers.
///
/// Parameters: cell size in pixels.
#[derive(Debug, Clone, Copy, Filter)]
#[filter(
    spatial,
    shader = "stylize/tiling/crystallize.wgsl",
    footprint_fn = crate::filters::footprint::crystallize
)]
pub struct Crystallize<T>(pub T);
