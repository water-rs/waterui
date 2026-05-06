//! Contrast filter implementation.

use crate::FilterDerive;

/// Adjusts the contrast of an image.
///
/// Applies the formula: `(color - 0.5) * amount + 0.5`
///
/// # Parameters
///
/// - `amount`: Contrast multiplier (0.0 = gray, 1.0 = unchanged, >1.0 = more contrast)
///
/// # Example
///
/// ```ignore
/// use filtrate::filters::Contrast;
///
/// let high_contrast = Contrast(1.5);
/// ```
#[derive(Debug, Clone, Copy, FilterDerive)]
#[filter(color_only, fragment = "fragments/contrast.wgsl")]
pub struct Contrast<T>(pub T);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn test_contrast_params() {
        let filter = Contrast(1.5f32);
        assert_eq!(filter.params(), [1.5]);
    }
}
