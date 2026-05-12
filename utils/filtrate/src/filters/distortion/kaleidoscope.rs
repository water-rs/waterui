//! Kaleidoscope filter implementation.

use crate::Filter;

/// Reflects content around repeated angular wedges.
#[derive(Debug, Clone, Filter)]
#[filter(spatial, shader = "distortion/kaleidoscope.wgsl")]
pub struct Kaleidoscope<T>(pub [T; 4]);
