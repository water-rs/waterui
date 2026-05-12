//! Mirror tile filter implementation.

use crate::Filter;

/// Repeats the image through mirrored tiling.
#[derive(Debug, Clone, Filter)]
#[filter(spatial, shader = "stylize/tiling/mirror_tile.wgsl")]
pub struct MirrorTile<T>(pub [T; 2]);
