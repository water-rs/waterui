//! Pixellate filter implementation.

use crate::FilterDerive;

/// Coalesces neighboring pixels into larger blocks.
#[derive(Debug, Clone, Copy, FilterDerive)]
#[filter(spatial, shader = "distortion/pixellate.wgsl")]
pub struct Pixellate<T>(pub T);

