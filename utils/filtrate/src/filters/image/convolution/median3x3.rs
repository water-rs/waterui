//! 3x3 median filter.

use crate::Filter;

/// Per-channel 3x3 median filter. Removes salt-and-pepper noise while
/// preserving edges better than a box blur.
///
/// # Example
///
/// ```ignore
/// use filtrate::filters::Median3x3;
///
/// let denoised = Median3x3;
/// ```
#[derive(Debug, Clone, Copy, Default, Filter)]
#[filter(spatial, shader = "image/convolution/median3x3.wgsl")]
pub struct Median3x3;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn median_is_spatial_with_zero_params() {
        assert!(!Median3x3::COLOR_ONLY);
        assert_eq!(Median3x3.params().len(), 0);
    }
}
