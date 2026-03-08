//! Crystallize filter implementation.

use crate::Filter;
use nami::Signal;

/// Creates a cell-like mosaic by snapping to jittered region centers.
#[derive(Debug, Clone, Copy)]
pub struct Crystallize<T>(pub T);

impl<T: Signal<Output = f32> + 'static> Filter for Crystallize<T> {
    const COLOR_ONLY: bool = false;

    type Params = [f32; 1];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 1] {
        [self.0.get()]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/crystallize.wgsl")
    }
}
