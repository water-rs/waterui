//! Gaussian blur filter implementation.

use crate::Filter;
use nami::Signal;

/// Applies separable gaussian blur.
#[derive(Debug, Clone, Copy)]
pub struct GaussianBlur<T>(pub T);

impl<T: Signal<Output = f32> + 'static> Filter for GaussianBlur<T> {
    const COLOR_ONLY: bool = false;

    type Params = [f32; 2];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 2] {
        let sigma = self.0.get();
        [sigma, sigma]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/gaussian_blur.wgsl")
    }
}
