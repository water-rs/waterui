//! 3x3 median filter.

use crate::Filter;

/// Per-channel 3x3 median filter. Removes salt-and-pepper noise while
/// preserving edges better than a box blur.
///
/// # Example
///
/// ```rust
/// # use filtrate::Filter;
/// use filtrate::filters::Median3x3;
///
/// let denoised = Median3x3;
/// # assert_eq!(denoised.params().len(), 0);
/// ```
#[derive(Debug, Clone, Copy, Default, Filter)]
#[filter(spatial, shader = "image/convolution/median3x3.wgsl", footprint = 1.0)]
pub struct Median3x3;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn median_is_spatial_with_zero_params() {
        assert_eq!(
            crate::SpatialFilter::footprint(&Median3x3),
            crate::Footprint::pixels(1.0)
        );
        assert_eq!(Median3x3.params().len(), 0);
    }
}
