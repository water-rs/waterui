//! Vignette filter implementation.

use crate::Filter;

/// Adds a vignette effect (darkened corners) to an image.
///
/// It depends on each pixel's position, which a colour stage cannot see, so
/// it is a spatial filter that reads only its own texel (footprint zero).
/// Distance is measured in isotropic space, so the vignette stays circular
/// on non-square images.
///
/// # Parameters
///
/// - `radius`: Inner radius where vignette starts (0.0-1.0)
/// - `softness`: How soft the vignette edge is (0.0-1.0)
///
/// # Example
///
/// ```rust
/// # use filtrate::Filter;
/// use filtrate::filters::Vignette;
///
/// let subtle = Vignette(0.8_f32, 0.3_f32);
/// let dramatic = Vignette(0.3_f32, 0.1_f32);
/// # assert_eq!(subtle.params(), [0.8, 0.3]);
/// ```
#[derive(Debug, Clone, Copy, Filter)]
#[filter(spatial, shader = "color/effect/vignette.wgsl", footprint = 0.0)]
pub struct Vignette<R, S>(pub R, pub S);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn test_vignette_params() {
        let filter = Vignette(0.5f32, 0.2f32);
        assert_eq!(filter.params(), [0.5, 0.2]);
        assert_eq!(
            crate::SpatialFilter::footprint(&filter),
            crate::Footprint::pixels(0.0)
        );
    }
}
