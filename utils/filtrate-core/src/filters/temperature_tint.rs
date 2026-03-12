//! Temperature/tint filter implementation.

use crate::Filter;
use nami::Signal;

/// Adjusts white balance through temperature and tint shifts.
///
/// # Parameters
///
/// - `temperature`: Blue↔yellow shift (-1.0 = cooler, 1.0 = warmer)
/// - `tint`: Green↔magenta shift (-1.0 = greener, 1.0 = more magenta)
#[derive(Debug, Clone, Copy)]
pub struct TemperatureTint<T, U>(pub T, pub U);

impl<T: Signal<Output = f32> + 'static, U: Signal<Output = f32> + 'static> Filter
    for TemperatureTint<T, U>
{
    const COLOR_ONLY: bool = true;

    type Params = [f32; 2];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 2] {
        [self.0.get(), self.1.get()]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/fragments/temperature_tint.wgsl")
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
