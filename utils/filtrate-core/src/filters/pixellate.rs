//! Pixellate filter implementation.

use crate::Filter;
use nami::Signal;

/// Coalesces neighboring pixels into larger blocks.
#[derive(Debug, Clone, Copy)]
pub struct Pixellate<T>(pub T);

impl<T: Signal<Output = f32> + 'static> Filter for Pixellate<T> {
    const COLOR_ONLY: bool = false;

    type Params = [f32; 1];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 1] {
        [self.0.get()]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/pixellate.wgsl")
    }
}
