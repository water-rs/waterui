//! Sepia filter implementation.

use crate::Filter;
use nami::Signal;

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
#[derive(Debug, Clone, Copy)]
pub struct Sepia<T>(pub T);

impl<T: Signal<Output = f32> + 'static> Filter for Sepia<T> {
    const COLOR_ONLY: bool = true;

    type Params = [f32; 1];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 1] {
        [self.0.get()]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/fragments/sepia.wgsl")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sepia_params() {
        let filter = Sepia(0.8f32);
        assert_eq!(filter.params(), [0.8]);
    }
}
