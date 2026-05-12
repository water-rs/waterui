//! Saturation filter implementation.

use crate::FilterDerive;

/// Adjusts the color saturation of an image.
///
/// Mixes between grayscale and the original color.
///
/// # Parameters
///
/// - `amount`: Saturation multiplier (0.0 = grayscale, 1.0 = unchanged, >1.0 = more saturated)
///
/// # Example
///
/// ```ignore
/// use filtrate::filters::Saturation;
///
/// let desaturated = Saturation(0.5);
/// let vibrant = Saturation(1.5);
/// ```
#[derive(Debug, Clone, Copy, FilterDerive)]
#[filter(color_only, fragment = "color/transform/saturation.wgsl")]
pub struct Saturation<T>(pub T);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn test_saturation_params() {
        let filter = Saturation(0.5f32);
        assert_eq!(filter.params(), [0.5]);
    }
}
