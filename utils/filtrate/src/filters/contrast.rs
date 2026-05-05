//! Contrast filter implementation.

use crate::Filter;
use nami::Signal;

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
#[derive(Debug, Clone, Copy)]
pub struct Contrast<T>(pub T);

impl<T: Signal<Output = f32> + 'static> Filter for Contrast<T> {
    const COLOR_ONLY: bool = true;

    type Params = [f32; 1];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 1] {
        [self.0.get()]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/fragments/contrast.wgsl")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contrast_params() {
        let filter = Contrast(1.5f32);
        assert_eq!(filter.params(), [1.5]);
    }
}
