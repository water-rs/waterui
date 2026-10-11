//! Pixellate filter implementation.

use crate::Filter;

/// Coalesces neighboring pixels into larger blocks.
///
/// Parameters: cell size in pixels.
#[derive(Debug, Clone, Copy, Filter)]
#[filter(
    spatial,
    shader = "distortion/pixellate.wgsl",
    footprint_fn = crate::filters::footprint::pixellate
)]
pub struct Pixellate<T>(pub T);
