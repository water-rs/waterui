//! Color matrix filter implementation.

use crate::Filter;

/// Applies a 3x4 colour matrix to straight-alpha RGB: three rows of four,
/// the fourth column a bias.
///
/// On premultiplied colour the bias scales with alpha, so the filter is a
/// linear map and [`LINEAR`](crate::ColorFilter::LINEAR). It carries a SIMD
/// CPU kernel.
#[derive(Debug, Clone, Filter)]
#[filter(
    color,
    shader = "color/transform/color_matrix.wgsl",
    linear = true,
    cpu = crate::cpu::color_matrix
)]
pub struct ColorMatrix<T>(pub [T; 12]);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn test_color_matrix_params() {
        let filter = ColorMatrix([
            1.0f32, 0.0, 0.0, 0.1, 0.0, 1.0, 0.0, 0.2, 0.0, 0.0, 1.0, 0.3,
        ]);
        assert_eq!(
            filter.params(),
            [1.0, 0.0, 0.0, 0.1, 0.0, 1.0, 0.0, 0.2, 0.0, 0.0, 1.0, 0.3]
        );
    }
}
