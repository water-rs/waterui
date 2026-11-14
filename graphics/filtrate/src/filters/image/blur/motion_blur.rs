//! Motion blur filter implementation.

use crate::Filter;

/// Applies directional motion blur, filtering each tap bilinearly.
///
/// # Parameters
///
/// - `radius`: Blur radius in pixels along the motion axis
/// - `angle`: Blur direction in degrees
#[derive(Debug, Clone, Copy, Filter)]
#[filter(
    spatial,
    shader = "image/blur/motion_blur.wgsl",
    footprint_fn = crate::filters::footprint::motion_blur
)]
pub struct MotionBlur<R, A>(pub R, pub A);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Filter, SpatialFilter};

    #[test]
    fn test_motion_blur_params() {
        let filter = MotionBlur(8.0f32, 45.0f32);
        assert_eq!(filter.params(), [8.0, 45.0]);
        // Eight taps each way, plus the bilinear neighbour.
        assert_eq!(filter.footprint(), crate::Footprint::pixels(9.0));
    }
}
