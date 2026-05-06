//! Temperature/tint filter implementation.

use crate::{Filter, FilterParam, SignalVisitor, StageCollector};

/// Adjusts white balance through temperature and tint shifts.
///
/// # Parameters
///
/// - `temperature`: Blue↔yellow shift (-1.0 = cooler, 1.0 = warmer)
/// - `tint`: Green↔magenta shift (-1.0 = greener, 1.0 = more magenta)
#[derive(Debug, Clone, Copy)]
pub struct TemperatureTint<T, U>(pub T, pub U);

impl<T: FilterParam, U: FilterParam> Filter
    for TemperatureTint<T, U>
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
        include_str!("../shaders/fragments/temperature_tint.wgsl")
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
    fn test_temperature_tint_params() {
        let filter = TemperatureTint(0.25f32, -0.15f32);
        assert_eq!(filter.params(), [0.25, -0.15]);
    }
}
