//! Zoom blur filter implementation.

use crate::Filter;

/// Applies radial zoom blur toward or away from a focal point.
///
/// The taps are displaced by a fraction of the image size, so the footprint
/// is unbounded in pixels once `amount` is non-zero.
///
/// # Parameters
///
/// - `amount`: Blur strength in normalized UV space
/// - `center_x`: Blur center x coordinate in normalized UV space
/// - `center_y`: Blur center y coordinate in normalized UV space
#[derive(Debug, Clone, Copy, Filter)]
#[filter(
    spatial,
    shader = "image/blur/zoom_blur.wgsl",
    footprint_fn = crate::filters::footprint::zoom_blur
)]
pub struct ZoomBlur<A, X, Y>(pub A, pub X, pub Y);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Filter, Footprint, SpatialFilter};

    #[test]
    fn test_zoom_blur_params() {
        let filter = ZoomBlur(0.2f32, 0.5f32, 0.5f32);
        assert_eq!(filter.params(), [0.2, 0.5, 0.5]);
        assert_eq!(
            filter.footprint(),
            Footprint::new(1.0, 0.2 * 1.5f32.hypot(1.5))
        );
        assert_eq!(
            ZoomBlur(0.0f32, 0.5f32, 0.5f32).footprint(),
            Footprint::ZERO
        );
    }
}
