//! Gamma filter implementation.

use crate::{Filter, FilterParam, SignalVisitor, StageCollector};

/// Adjusts image gamma.
///
/// # Parameters
///
/// - `gamma`: Gamma exponent (>0.0, 1.0 = unchanged)
#[derive(Debug, Clone, Copy)]
pub struct Gamma<T>(pub T);

impl<T: FilterParam> Filter for Gamma<T> {
    const COLOR_ONLY: bool = true;

    type Params = [f32; 1];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 1] {
        [self.0.snapshot()]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/fragments/gamma.wgsl")
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.color_fragment(self.fragments(), 1);
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        v.visit(0, &self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gamma_params() {
        let filter = Gamma(2.2f32);
        assert_eq!(filter.params(), [2.2]);
    }
}
