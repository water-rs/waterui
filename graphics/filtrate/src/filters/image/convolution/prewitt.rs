//! Prewitt edge detection filter.

use crate::Filter;

/// Applies a 3x3 Prewitt operator (uniform-weight 3x3 kernels) to the
/// working-space luma and outputs gradient magnitude as a grayscale image.
///
/// Compared with [`Sobel`](crate::filters::Sobel), Prewitt weights all
/// neighbour samples equally; the resulting edges are slightly noisier but
/// faster to reason about for non-photographic content.
///
/// # Example
///
/// ```rust
/// # use filtrate::Filter;
/// use filtrate::filters::Prewitt;
///
/// let edges = Prewitt;
/// # assert_eq!(edges.params().len(), 0);
/// ```
#[derive(Debug, Clone, Copy, Default, Filter)]
#[filter(
    spatial,
    shader = "image/convolution/gradient.wgsl",
    footprint = 1.0,
    constants = [1.0, 1.0, 1.0]
)]
pub struct Prewitt;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn prewitt_is_spatial_with_zero_params() {
        assert_eq!(
            crate::SpatialFilter::footprint(&Prewitt),
            crate::Footprint::pixels(1.0)
        );
        assert_eq!(Prewitt.params().len(), 0);
    }
}
