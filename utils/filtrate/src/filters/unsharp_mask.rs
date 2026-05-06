//! Unsharp mask filter implementation.

use crate::FilterDerive;

/// Sharpens image detail using an unsharp mask.
#[derive(Debug, Clone, FilterDerive)]
#[filter(spatial, shader = "unsharp_mask.wgsl")]
pub struct UnsharpMask<T>(pub [T; 2]);

