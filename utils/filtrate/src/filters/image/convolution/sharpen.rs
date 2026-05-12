//! Sharpen filter implementation.

use crate::Filter;

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
#[derive(Debug, Clone, Copy, Filter)]
#[filter(spatial, shader = "image/convolution/sharpen.wgsl")]
pub struct Sharpen<T>(pub T);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

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
