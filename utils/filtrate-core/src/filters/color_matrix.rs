//! Color matrix filter implementation.

use crate::Filter;
use nami::Signal;

/// Applies a 3x4 color matrix to RGB channels.
#[derive(Debug, Clone)]
pub struct ColorMatrix<T>(pub [T; 12]);

impl<T> Filter for ColorMatrix<T>
where
    T: Signal<Output = f32> + Clone + 'static,
{
    const COLOR_ONLY: bool = true;

    type Params = [f32; 12];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 12] {
        core::array::from_fn(|idx| self.0[idx].get())
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/fragments/color_matrix.wgsl")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
