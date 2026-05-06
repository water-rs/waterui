//! Exposure filter implementation.

use crate::{Filter, FilterParam, SignalVisitor, StageCollector};

/// Adjusts exposure in photographic stops.
///
/// Positive values brighten, negative values darken.
///
/// # Parameters
///
/// - `ev`: Exposure value in stops (0.0 = unchanged, 1.0 = +1 stop, -1.0 = -1 stop)
#[derive(Debug, Clone, Copy)]
pub struct Exposure<T>(pub T);

impl<T: FilterParam> Filter for Exposure<T> {
    const COLOR_ONLY: bool = true;

    type Params = [f32; 1];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 1] {
        [self.0.snapshot()]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/fragments/exposure.wgsl")
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
    fn test_exposure_params() {
        let filter = Exposure(1.5f32);
        assert_eq!(filter.params(), [1.5]);
    }
}
