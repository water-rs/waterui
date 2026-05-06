//! Gaussian blur filter implementation.

use crate::{Filter, FilterParam, SignalVisitor, StageCollector};

/// Applies separable gaussian blur.
#[derive(Debug, Clone, Copy)]
pub struct GaussianBlur<T>(pub T);

impl<T: FilterParam> Filter for GaussianBlur<T> {
    const COLOR_ONLY: bool = false;

    type Params = [f32; 2];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 2] {
        let sigma = self.0.snapshot();
        [sigma, sigma]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/gaussian_blur_horizontal.wgsl")
    }

    fn pass_count(&self) -> u32 {
        2
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial_shader(include_str!("../shaders/gaussian_blur_horizontal.wgsl"), 1);
        c.spatial_shader(include_str!("../shaders/gaussian_blur_vertical.wgsl"), 1);
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        // Sigma drives both separable passes; broadcast to indices 0 and 1.
        v.visit(0, &self.0);
        v.visit(1, &self.0);
    }
}
