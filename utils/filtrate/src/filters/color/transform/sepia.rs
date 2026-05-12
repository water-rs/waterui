//! Sepia filter implementation.

use crate::Filter;

/// Applies a sepia tone effect to an image.
///
/// Gives the image a warm, vintage appearance.
///
/// # Parameters
///
/// - `intensity`: Sepia intensity (0.0 = original, 1.0 = full sepia)
///
/// # Example
///
/// ```ignore
/// use filtrate::filters::Sepia;
///
/// let vintage = Sepia(0.8);
/// ```
#[derive(Debug, Clone, Copy, Filter)]
#[filter(color_only, fragment = "color/transform/sepia.wgsl")]
pub struct Sepia<T>(pub T);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn test_sepia_params() {
        let filter = Sepia(0.8f32);
        assert_eq!(filter.params(), [0.8]);
    }
}
