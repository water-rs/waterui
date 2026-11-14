//! Perspective correction filter implementation.

use crate::Filter;

/// Corrects a perspective-skewed quadrilateral back to a rectangle.
///
/// Parameters: the quad's top-left, top-right, bottom-right and bottom-left
/// corners, x then y, in uv. It shares its stage with
/// [`PerspectiveTransform`](crate::filters::PerspectiveTransform), which
/// specializes the other direction. Any pixel can read any other, so the
/// footprint spans the whole image extent.
#[derive(Debug, Clone, Filter)]
#[filter(
    spatial,
    shader = "distortion/perspective.wgsl",
    footprint_extent = 1.0,
    constants = [0.0]
)]
pub struct PerspectiveCorrection<T>(pub [T; 8]);
