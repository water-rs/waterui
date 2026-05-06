//! Grayscale filter implementation.

use crate::{Filter, FilterParam, SignalVisitor, StageCollector};

/// Converts an image to grayscale.
///
/// Uses luminance-based conversion with configurable intensity.
///
/// # Parameters
///
/// - `intensity`: Mix factor (0.0 = original, 1.0 = full grayscale)
///
/// # Example
///
/// ```ignore
/// use filtrate::filters::Grayscale;
///
/// let full_gray = Grayscale(1.0);
/// let partial = Grayscale(0.5);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Grayscale<T>(pub T);

impl<T: FilterParam> Filter for Grayscale<T> {
    const COLOR_ONLY: bool = true;

    type Params = [f32; 1];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 1] {
        [self.0.snapshot()]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/fragments/grayscale.wgsl")
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
    fn test_grayscale_params() {
        let filter = Grayscale(1.0f32);
        assert_eq!(filter.params(), [1.0]);
    }
}
