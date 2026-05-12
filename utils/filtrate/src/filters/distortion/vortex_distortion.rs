//! Vortex distortion filter implementation.

use crate::Filter;

/// Applies a vortex-style spiral distortion.
#[derive(Debug, Clone, Filter)]
#[filter(spatial, shader = "distortion/vortex_distortion.wgsl")]
pub struct VortexDistortion<T>(pub [T; 4]);
