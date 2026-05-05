//! Mirror tile filter implementation.

use crate::Filter;
use nami::Signal;

/// Repeats the image through mirrored tiling.
#[derive(Debug, Clone)]
pub struct MirrorTile<T>(pub [T; 2]);

impl<T> Filter for MirrorTile<T>
where
    T: Signal<Output = f32> + Clone + 'static,
{
    const COLOR_ONLY: bool = false;

    type Params = [f32; 2];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 2] {
        core::array::from_fn(|idx| self.0[idx].get())
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/mirror_tile.wgsl")
    }
}
