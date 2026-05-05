//! Blur filter implementation.

use crate::Filter;
use nami::Signal;

/// Applies a box blur effect to an image.
///
/// This is a spatial filter that samples neighboring pixels, so it
/// cannot be fused with other filters. It requires its own GPU pass.
///
/// # Parameters
///
/// - `radius`: Blur radius in pixels (0.0 = no blur, higher = more blur)
///
/// # Example
///
/// ```ignore
/// use filtrate::filters::Blur;
///
/// let soft = Blur(5.0);
/// let heavy = Blur(20.0);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Blur<T>(pub T);

impl<T: Signal<Output = f32> + 'static> Filter for Blur<T> {
    /// Blur samples neighboring pixels, so it cannot be fused.
    const COLOR_ONLY: bool = false;

    // Separable two-pass blur uses the same radius in both passes.
    type Params = [f32; 2];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 2] {
        let radius = self.0.get();
        [radius, radius]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        // Note: Blur uses a standalone shader, not a fragment
        // The pipeline handles this differently for spatial filters
        include_str!("../shaders/blur.wgsl")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blur_params() {
        let filter = Blur(10.0f32);
        assert_eq!(filter.params(), [10.0, 10.0]);
    }

    #[test]
    fn test_blur_not_color_only() {
        assert!(!Blur::<f32>::COLOR_ONLY);
    }
}
