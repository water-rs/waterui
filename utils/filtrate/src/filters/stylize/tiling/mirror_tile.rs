//! Mirror tile filter implementation.

use crate::FilterDerive;

/// Repeats the image through mirrored tiling.
#[derive(Debug, Clone, FilterDerive)]
#[filter(spatial, shader = "stylize/tiling/mirror_tile.wgsl")]
pub struct MirrorTile<T>(pub [T; 2]);

