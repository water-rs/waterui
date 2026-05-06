//! Vignette filter implementation.

use crate::{Filter, FilterParam, SignalVisitor, StageCollector};

/// Adds a vignette effect (darkened corners) to an image.
///
/// # Parameters
///
/// - `radius`: Inner radius where vignette starts (0.0-1.0)
/// - `softness`: How soft the vignette edge is (0.0-1.0)
///
/// # Example
///
/// ```ignore
/// use filtrate::filters::Vignette;
///
/// let subtle = Vignette(0.8, 0.3);
/// let dramatic = Vignette(0.3, 0.1);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Vignette<R, S>(pub R, pub S);

impl<R: FilterParam, S: FilterParam> Filter
    for Vignette<R, S>
{
    const COLOR_ONLY: bool = true;

    type Params = [f32; 2];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 2] {
        [self.0.snapshot(), self.1.snapshot()]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/fragments/vignette.wgsl")
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.color_fragment(self.fragments(), 2);
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        v.visit(0, &self.0);
        v.visit(1, &self.1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vignette_params() {
        let filter = Vignette(0.5f32, 0.2f32);
        assert_eq!(filter.params(), [0.5, 0.2]);
    }
}
