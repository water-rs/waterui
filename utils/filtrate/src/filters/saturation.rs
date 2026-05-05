//! Saturation filter implementation.

use crate::Filter;
use nami::Signal;

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
#[derive(Debug, Clone, Copy)]
pub struct Saturation<T>(pub T);

impl<T: Signal<Output = f32> + 'static> Filter for Saturation<T> {
    const COLOR_ONLY: bool = true;

    type Params = [f32; 1];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 1] {
        [self.0.get()]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/fragments/saturation.wgsl")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_saturation_params() {
        let filter = Saturation(0.5f32);
        assert_eq!(filter.params(), [0.5]);
    }
}
