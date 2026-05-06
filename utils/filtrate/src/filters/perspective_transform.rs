//! Perspective transform filter implementation.

use crate::{Filter, FilterParam, SignalVisitor, StageCollector};

/// Maps a source quadrilateral into the output rectangle.
#[derive(Debug, Clone)]
pub struct PerspectiveTransform<T>(pub [T; 8]);

impl<T> Filter for PerspectiveTransform<T>
where
    T: FilterParam + Clone,
{
    const COLOR_ONLY: bool = false;

    type Params = [f32; 8];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 8] {
        core::array::from_fn(|idx| self.0[idx].snapshot())
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/perspective_transform.wgsl")
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial_shader(self.fragments(), 8);
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        for (i, p) in self.0.iter().enumerate() {
            v.visit(i, p);
        }
    }
}
