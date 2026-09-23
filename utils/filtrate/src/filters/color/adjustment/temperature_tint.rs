//! Temperature/tint filter implementation.

use crate::Filter;

/// Adjusts white balance through temperature and tint shifts.
///
/// # Parameters
///
/// - `temperature`: Blue↔yellow shift (-1.0 = cooler, 1.0 = warmer)
/// - `tint`: Green↔magenta shift (-1.0 = greener, 1.0 = more magenta)
#[derive(Debug, Clone, Copy, Filter)]
#[filter(color_only, fragment = "color/adjustment/temperature_tint.wgsl")]
pub struct TemperatureTint<T, U>(pub T, pub U);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn test_temperature_tint_params() {
        let filter = TemperatureTint(0.25f32, -0.15f32);
        assert_eq!(filter.params(), [0.25, -0.15]);
    }
}
