//! Edge work filter implementation.

use crate::Filter;

/// Highlights local edges using a Sobel-style gradient magnitude.
///
/// Parameters: the sampling radius in pixels (at least one) and the amount
/// the magnitude is scaled by. It is the shared gradient stage with centre
/// weight 2.
#[derive(Debug, Clone, Filter)]
#[filter(
    spatial,
    shader = "image/convolution/gradient.wgsl",
    footprint_fn = crate::filters::footprint::edge_work,
    constants = [2.0]
)]
pub struct EdgeWork<T>(pub [T; 2]);
