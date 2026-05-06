//! Crystallize filter implementation.

use crate::{Filter, FilterParam, SignalVisitor, StageCollector};

/// Creates a cell-like mosaic by snapping to jittered region centers.
#[derive(Debug, Clone, Copy)]
pub struct Crystallize<T>(pub T);

impl<T: FilterParam> Filter for Crystallize<T> {
    const COLOR_ONLY: bool = false;

    type Params = [f32; 1];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 1] {
        [self.0.snapshot()]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/crystallize.wgsl")
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial_shader(self.fragments(), 1);
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        v.visit(0, &self.0);
    }
}
