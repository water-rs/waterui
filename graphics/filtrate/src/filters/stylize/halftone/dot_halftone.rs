//! Dot halftone filter implementation.

use crate::Filter;

/// Renders the working-space luma as a dot-screen halftone pattern.
///
/// Parameters: cell size in pixels, screen angle in degrees, and the screen
/// centre x and y in uv. Each pixel reads only its own texel.
#[derive(Debug, Clone, Filter)]
#[filter(
    spatial,
    shader = "stylize/halftone/dot_halftone.wgsl",
    footprint = 0.0
)]
pub struct DotHalftone<T>(pub [T; 4]);
