//! Perspective correction filter implementation.

use crate::Filter;
use nami::Signal;

/// Corrects a perspective-skewed quadrilateral back to a rectangle.
#[derive(Debug, Clone)]
pub struct PerspectiveCorrection<T>(pub [T; 8]);

impl<T> Filter for PerspectiveCorrection<T>
where
    T: Signal<Output = f32> + Clone + 'static,
{
    const COLOR_ONLY: bool = false;

    type Params = [f32; 8];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 8] {
        core::array::from_fn(|idx| self.0[idx].get())
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/perspective_correction.wgsl")
    }
}
