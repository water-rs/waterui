//! Sobel edge detection filter.

use crate::FilterDerive;

/// Applies a 3x3 Sobel operator to the luminance channel and outputs
/// gradient magnitude as a grayscale image. Useful for edge highlighting.
///
/// # Example
///
/// ```ignore
/// use filtrate::filters::Sobel;
///
/// let edges = Sobel;
/// ```
#[derive(Debug, Clone, Copy, Default, FilterDerive)]
#[filter(spatial, shader = "image/convolution/sobel.wgsl")]
pub struct Sobel;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn sobel_is_spatial_with_zero_params() {
        assert!(!Sobel::COLOR_ONLY);
        assert_eq!(Sobel.params().len(), 0);
    }
}
