//! Zoom blur filter implementation.

use crate::{Filter, FilterParam, SignalVisitor, StageCollector};

/// Applies radial zoom blur toward or away from a focal point.
///
/// # Parameters
///
/// - `amount`: Blur strength in normalized UV space
/// - `center_x`: Blur center x coordinate in normalized UV space
/// - `center_y`: Blur center y coordinate in normalized UV space
#[derive(Debug, Clone, Copy)]
pub struct ZoomBlur<A, X, Y>(pub A, pub X, pub Y);

impl<A, X, Y> Filter for ZoomBlur<A, X, Y>
where
    A: FilterParam,
    X: FilterParam,
    Y: FilterParam,
{
    const COLOR_ONLY: bool = false;

    type Params = [f32; 3];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 3] {
        [self.0.snapshot(), self.1.snapshot(), self.2.snapshot()]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/zoom_blur.wgsl")
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial_shader(self.fragments(), 3);
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        v.visit(0, &self.0);
        v.visit(1, &self.1);
        v.visit(2, &self.2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zoom_blur_params() {
        let filter = ZoomBlur(0.2f32, 0.5f32, 0.5f32);
        assert_eq!(filter.params(), [0.2, 0.5, 0.5]);
    }

    #[test]
    fn test_zoom_blur_not_color_only() {
        assert!(!ZoomBlur::<f32, f32, f32>::COLOR_ONLY);
    }
}
