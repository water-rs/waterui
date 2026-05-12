//! Grayscale filter implementation.

use crate::FilterDerive;

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
/// ```ignore
/// use filtrate::filters::Grayscale;
///
/// let full_gray = Grayscale(1.0);
/// let partial = Grayscale(0.5);
/// ```
#[derive(Debug, Clone, Copy, FilterDerive)]
#[filter(color_only, fragment = "color/transform/grayscale.wgsl")]
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
