//! Grayscale filter implementation.

use crate::Filter;

/// Converts an image to grayscale.
///
/// Mixes toward the working-space luma with configurable intensity, a
/// linear map, so the filter is [`LINEAR`](crate::ColorFilter::LINEAR). It
/// carries a SIMD CPU kernel.
///
/// # Parameters
///
/// - `intensity`: Mix factor (0.0 = original, 1.0 = full grayscale)
///
/// # Example
///
/// ```rust
/// # use filtrate::Filter;
/// use filtrate::filters::Grayscale;
///
/// let full_gray = Grayscale(1.0_f32);
/// let partial = Grayscale(0.5_f32);
/// # assert_eq!(full_gray.params(), [1.0]);
/// # assert_eq!(partial.params(), [0.5]);
/// ```
#[derive(Debug, Clone, Copy, Filter)]
#[filter(
    color,
    shader = "color/transform/grayscale.wgsl",
    linear = true,
    cpu = crate::cpu::grayscale
)]
pub struct Grayscale<T>(pub T);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn test_grayscale_params() {
        let filter = Grayscale(1.0f32);
        assert_eq!(filter.params(), [1.0]);
    }
}
