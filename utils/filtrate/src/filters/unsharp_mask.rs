//! Unsharp mask filter implementation.

use crate::{Filter, FilterParam, SignalVisitor, StageCollector};

/// Sharpens image detail using an unsharp mask.
#[derive(Debug, Clone)]
pub struct UnsharpMask<T>(pub [T; 2]);

impl<T> Filter for UnsharpMask<T>
where
    T: FilterParam + Clone,
{
    const COLOR_ONLY: bool = false;

    type Params = [f32; 2];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 2] {
        core::array::from_fn(|idx| self.0[idx].snapshot())
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/unsharp_mask.wgsl")
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial_shader(self.fragments(), 2);
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        for (i, p) in self.0.iter().enumerate() {
            v.visit(i, p);
        }
    }
}
