//! Bloom filter implementation.

use crate::{Filter, FilterParam, SignalVisitor, StageCollector};

/// Adds a bright glow around high-luminance regions.
#[derive(Debug, Clone)]
pub struct Bloom<T>(pub [T; 3]);

impl<T> Filter for Bloom<T>
where
    T: FilterParam + Clone,
{
    const COLOR_ONLY: bool = false;

    type Params = [f32; 3];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 3] {
        core::array::from_fn(|idx| self.0[idx].snapshot())
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/bloom.wgsl")
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial_shader(self.fragments(), 3);
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        for (i, p) in self.0.iter().enumerate() {
            v.visit(i, p);
        }
    }
}
