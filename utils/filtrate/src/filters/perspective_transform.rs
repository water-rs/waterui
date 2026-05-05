//! Perspective transform filter implementation.

use crate::Filter;
use nami::Signal;

/// Maps a source quadrilateral into the output rectangle.
#[derive(Debug, Clone)]
pub struct PerspectiveTransform<T>(pub [T; 8]);

impl<T> Filter for PerspectiveTransform<T>
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
        include_str!("../shaders/perspective_transform.wgsl")
    }
}
