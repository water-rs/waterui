//! 5x5 convolution filter with a caller-supplied kernel.

use crate::Filter;

/// Per-channel 5x5 convolution with an arbitrary 25-element kernel
/// (row-major, top-left to bottom-right).
///
/// Counterpart to [`Convolution3x3`](crate::filters::Convolution3x3) when a
/// wider support is needed (gaussian approximations, larger emboss, custom
/// blurs).
///
/// Note: 25 floats consume nearly half of the 64-float pipeline parameter
/// budget — chaining a `Convolution5x5` with another large filter may
/// exceed the limit and fail setup at runtime.
#[derive(Debug, Clone, Filter)]
#[filter(spatial, shader = "image/convolution/convolution5x5.wgsl")]
pub struct Convolution5x5<T>(pub [T; 25]);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn convolution5x5_param_count() {
        let mut k = [0.0_f32; 25];
        k[12] = 1.0;
        let identity = Convolution5x5(k);
        assert_eq!(identity.params().len(), 25);
        const { assert!(!Convolution5x5::<f32>::COLOR_ONLY) };
    }
}
