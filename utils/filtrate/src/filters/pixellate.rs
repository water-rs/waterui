//! Pixellate filter implementation.

use crate::{Filter, FilterParam, SignalVisitor, StageCollector};

/// Coalesces neighboring pixels into larger blocks.
#[derive(Debug, Clone, Copy)]
pub struct Pixellate<T>(pub T);

impl<T: FilterParam> Filter for Pixellate<T> {
    const COLOR_ONLY: bool = false;

    type Params = [f32; 1];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 1] {
        [self.0.snapshot()]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/pixellate.wgsl")
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial_shader(self.fragments(), 1);
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        v.visit(0, &self.0);
    }
}
