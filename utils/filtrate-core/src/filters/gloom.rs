//! Gloom filter implementation.

use crate::Filter;
use nami::Signal;

/// Adds a dark halo around high-luminance regions.
#[derive(Debug, Clone)]
pub struct Gloom<T>(pub [T; 3]);

impl<T> Filter for Gloom<T>
where
    T: Signal<Output = f32> + Clone + 'static,
{
    const COLOR_ONLY: bool = false;

    type Params = [f32; 3];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 3] {
        core::array::from_fn(|idx| self.0[idx].get())
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/gloom.wgsl")
    }
}
