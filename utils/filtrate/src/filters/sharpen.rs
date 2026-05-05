//! Sharpen filter implementation.

use crate::Filter;
use nami::Signal;

/// Sharpens image details using an unsharp mask.
///
/// This is a spatial filter that samples neighboring pixels, so it
/// cannot be fused with other filters. It requires its own GPU pass.
///
/// # Parameters
///
/// - `amount`: Sharpening strength (0.0 = unchanged, 1.0 = normal, >1.0 = more sharp)
///
/// # Example
///
/// ```ignore
/// use filtrate::filters::Sharpen;
///
/// let subtle = Sharpen(0.5);
/// let crisp = Sharpen(1.5);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Sharpen<T>(pub T);

impl<T: Signal<Output = f32> + 'static> Filter for Sharpen<T> {
    /// Sharpen samples neighboring pixels, so it cannot be fused.
    const COLOR_ONLY: bool = false;

    type Params = [f32; 1];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 1] {
        [self.0.get()]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        // Note: Sharpen uses a standalone shader, not a fragment
        // The pipeline handles this differently for spatial filters
        include_str!("../shaders/sharpen.wgsl")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sharpen_params() {
        let filter = Sharpen(1.0f32);
        assert_eq!(filter.params(), [1.0]);
    }

    #[test]
    fn test_sharpen_not_color_only() {
        assert!(!Sharpen::<f32>::COLOR_ONLY);
    }
}
