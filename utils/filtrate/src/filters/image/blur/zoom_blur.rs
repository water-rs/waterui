//! Zoom blur filter implementation.

use crate::Filter;

/// Applies radial zoom blur toward or away from a focal point.
///
/// # Parameters
///
/// - `amount`: Blur strength in normalized UV space
/// - `center_x`: Blur center x coordinate in normalized UV space
/// - `center_y`: Blur center y coordinate in normalized UV space
#[derive(Debug, Clone, Copy, Filter)]
#[filter(spatial, shader = "image/blur/zoom_blur.wgsl")]
pub struct ZoomBlur<A, X, Y>(pub A, pub X, pub Y);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

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
