//! Vortex distortion filter implementation.

use crate::Filter;
use nami::Signal;

/// Applies a vortex-style spiral distortion.
#[derive(Debug, Clone)]
pub struct VortexDistortion<T>(pub [T; 4]);

impl<T> Filter for VortexDistortion<T>
where
    T: Signal<Output = f32> + Clone + 'static,
{
    const COLOR_ONLY: bool = false;

    type Params = [f32; 4];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 4] {
        core::array::from_fn(|idx| self.0[idx].get())
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/vortex_distortion.wgsl")
    }
}
