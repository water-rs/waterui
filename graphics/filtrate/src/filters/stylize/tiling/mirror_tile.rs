//! Mirror tile filter implementation.

use crate::Filter;

/// Repeats the image through mirrored tiling.
///
/// Parameters: the repeat counts along x and y. Any pixel can read any
/// other, so the footprint spans the whole image extent.
#[derive(Debug, Clone, Filter)]
#[filter(
    spatial,
    shader = "stylize/tiling/mirror_tile.wgsl",
    footprint_extent = 1.0
)]
pub struct MirrorTile<T>(pub [T; 2]);
