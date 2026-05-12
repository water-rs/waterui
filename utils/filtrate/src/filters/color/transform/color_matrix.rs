//! Color matrix filter implementation.

use crate::Filter;

/// Applies a 3x4 color matrix to RGB channels.
#[derive(Debug, Clone, Filter)]
#[filter(color_only, fragment = "color/transform/color_matrix.wgsl")]
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
