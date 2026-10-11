//! Line halftone filter implementation.

use crate::Filter;

/// Renders the working-space luma as an angled line-screen halftone pattern.
///
/// Parameters: line pitch in pixels, screen angle in degrees, and the screen
/// centre x and y in uv. Each pixel reads only its own texel.
#[derive(Debug, Clone, Filter)]
#[filter(
    spatial,
    shader = "stylize/halftone/line_halftone.wgsl",
    footprint = 0.0
)]
pub struct LineHalftone<T>(pub [T; 4]);
