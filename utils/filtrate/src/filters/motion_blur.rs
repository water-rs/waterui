//! Motion blur filter implementation.

use crate::{Filter, FilterParam, SignalVisitor, StageCollector};

/// Applies directional motion blur.
///
/// # Parameters
///
/// - `radius`: Blur radius in pixels along the motion axis
/// - `angle`: Blur direction in degrees
#[derive(Debug, Clone, Copy)]
pub struct MotionBlur<R, A>(pub R, pub A);

impl<R: FilterParam, A: FilterParam> Filter
    for MotionBlur<R, A>
{
    const COLOR_ONLY: bool = false;

    type Params = [f32; 2];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 2] {
        [self.0.snapshot(), self.1.snapshot()]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/motion_blur.wgsl")
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial_shader(self.fragments(), 2);
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        v.visit(0, &self.0);
        v.visit(1, &self.1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
