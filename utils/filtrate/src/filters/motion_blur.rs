//! Motion blur filter implementation.

use crate::FilterDerive;

/// Applies directional motion blur.
///
/// # Parameters
///
/// - `radius`: Blur radius in pixels along the motion axis
/// - `angle`: Blur direction in degrees
#[derive(Debug, Clone, Copy, FilterDerive)]
#[filter(spatial, shader = "motion_blur.wgsl")]
pub struct MotionBlur<R, A>(pub R, pub A);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn test_motion_blur_params() {
        let filter = MotionBlur(8.0f32, 45.0f32);
        assert_eq!(filter.params(), [8.0, 45.0]);
    }

    #[test]
    fn test_motion_blur_not_color_only() {
        assert!(!MotionBlur::<f32, f32>::COLOR_ONLY);
    }
}
