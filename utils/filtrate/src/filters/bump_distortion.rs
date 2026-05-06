//! Bump distortion filter implementation.

use crate::{Filter, FilterParam, SignalVisitor, StageCollector};

/// Applies convex/concave bump distortion around a center.
#[derive(Debug, Clone)]
pub struct BumpDistortion<T>(pub [T; 4]);

impl<T> Filter for BumpDistortion<T>
where
    T: FilterParam + Clone,
{
    const COLOR_ONLY: bool = false;

    type Params = [f32; 4];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 4] {
        core::array::from_fn(|idx| self.0[idx].snapshot())
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/bump_distortion.wgsl")
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial_shader(self.fragments(), 4);
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        for (i, p) in self.0.iter().enumerate() {
            v.visit(i, p);
        }
    }
}
