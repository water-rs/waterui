//! Vortex distortion filter implementation.

use crate::FilterDerive;

/// Applies a vortex-style spiral distortion.
#[derive(Debug, Clone, FilterDerive)]
#[filter(spatial, shader = "distortion/vortex_distortion.wgsl")]
pub struct VortexDistortion<T>(pub [T; 4]);

