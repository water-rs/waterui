//! Grayscale filter implementation.

use crate::Filter;

/// Converts an image to grayscale.
///
/// Uses luminance-based conversion with configurable intensity.
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
#[filter(color_only, shader = "color/transform/grayscale.wgsl")]
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
