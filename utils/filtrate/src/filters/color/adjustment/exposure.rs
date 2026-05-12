//! Exposure filter implementation.

use crate::FilterDerive;

/// Adjusts exposure in photographic stops.
///
/// Positive values brighten, negative values darken.
///
/// # Parameters
///
/// - `ev`: Exposure value in stops (0.0 = unchanged, 1.0 = +1 stop, -1.0 = -1 stop)
#[derive(Debug, Clone, Copy, FilterDerive)]
#[filter(color_only, fragment = "color/adjustment/exposure.wgsl")]
pub struct Exposure<T>(pub T);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn test_exposure_params() {
        let filter = Exposure(1.5f32);
        assert_eq!(filter.params(), [1.5]);
    }
}
