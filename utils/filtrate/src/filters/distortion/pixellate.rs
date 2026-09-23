//! Pixellate filter implementation.

use crate::Filter;

/// Coalesces neighboring pixels into larger blocks.
#[derive(Debug, Clone, Copy, Filter)]
#[filter(spatial, shader = "distortion/pixellate.wgsl")]
pub struct Pixellate<T>(pub T);
