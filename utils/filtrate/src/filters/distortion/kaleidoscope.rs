//! Kaleidoscope filter implementation.

use crate::FilterDerive;

/// Reflects content around repeated angular wedges.
#[derive(Debug, Clone, FilterDerive)]
#[filter(spatial, shader = "distortion/kaleidoscope.wgsl")]
pub struct Kaleidoscope<T>(pub [T; 4]);

