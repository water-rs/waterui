//! Prewitt edge detection filter.

use crate::Filter;

/// Applies a 3x3 Prewitt operator (uniform-weight 3x3 kernels) to the
/// luminance channel and outputs gradient magnitude as a grayscale image.
///
/// Compared with [`Sobel`](crate::filters::Sobel), Prewitt weights all
/// neighbour samples equally; the resulting edges are slightly noisier but
/// faster to reason about for non-photographic content.
///
/// # Example
///
/// ```ignore
/// use filtrate::filters::Prewitt;
///
/// let edges = Prewitt;
/// ```
#[derive(Debug, Clone, Copy, Default, Filter)]
#[filter(spatial, shader = "image/convolution/prewitt.wgsl")]
pub struct Prewitt;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn prewitt_is_spatial_with_zero_params() {
        assert!(!Prewitt::COLOR_ONLY);
        assert_eq!(Prewitt.params().len(), 0);
    }
}
