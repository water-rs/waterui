//! Sobel edge detection filter.

use crate::Filter;

/// Applies a 3x3 Sobel operator to the luminance channel and outputs
/// gradient magnitude as a grayscale image. Useful for edge highlighting.
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
#[filter(spatial, shader = "image/convolution/sobel.wgsl")]
pub struct Sobel;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn sobel_is_spatial_with_zero_params() {
        const { assert!(!Sobel::COLOR_ONLY) };
        assert_eq!(Sobel.params().len(), 0);
    }
}
