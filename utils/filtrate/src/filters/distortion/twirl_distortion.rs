//! Twirl distortion filter implementation.

use crate::FilterDerive;

/// Applies a twirl distortion around a center point.
#[derive(Debug, Clone, FilterDerive)]
#[filter(spatial, shader = "distortion/twirl_distortion.wgsl")]
pub struct TwirlDistortion<T>(pub [T; 4]);

