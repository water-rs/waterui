//! 3x3 convolution filter with a caller-supplied kernel.

use crate::Filter;

/// Per-channel 3x3 convolution with an arbitrary 9-element kernel
/// (row-major, top-left to bottom-right).
///
/// The kernel is not auto-normalised — callers choose weights that match
/// their intent (edge enhance, blur, emboss, etc.).
///
/// # Example
///
/// ```ignore
/// use filtrate::filters::Convolution3x3;
///
/// // Sharpen
/// let sharpen = Convolution3x3([
///      0.0_f32, -1.0, 0.0,
///     -1.0,      5.0, -1.0,
///      0.0,     -1.0, 0.0,
/// ]);
/// ```
#[derive(Debug, Clone, Filter)]
#[filter(spatial, shader = "image/convolution/convolution3x3.wgsl")]
pub struct Convolution3x3<T>(pub [T; 9]);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn convolution3x3_param_count() {
        let identity = Convolution3x3([0.0_f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(identity.params().len(), 9);
        assert!(!Convolution3x3::<f32>::COLOR_ONLY);
    }
}
