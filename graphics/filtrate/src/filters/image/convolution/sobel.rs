//! Sobel edge detection filter.

use crate::Filter;

/// Applies a 3x3 Sobel operator to the working-space luma and outputs the
/// gradient magnitude as a grayscale image. Useful for edge highlighting.
///
/// It is the shared gradient stage specialized to radius 1, amount 1 and
/// centre weight 2.
///
/// # Example
///
/// ```rust
/// # use filtrate::Filter;
/// use filtrate::filters::Sobel;
///
/// let edges = Sobel;
/// # assert_eq!(edges.params().len(), 0);
/// ```
#[derive(Debug, Clone, Copy, Default, Filter)]
#[filter(
    spatial,
    shader = "image/convolution/gradient.wgsl",
    footprint = 1.0,
    constants = [1.0, 1.0, 2.0]
)]
pub struct Sobel;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn sobel_is_spatial_with_zero_params() {
        assert_eq!(
            crate::SpatialFilter::footprint(&Sobel),
            crate::Footprint::pixels(1.0)
        );
        assert_eq!(Sobel.params().len(), 0);
    }
}
