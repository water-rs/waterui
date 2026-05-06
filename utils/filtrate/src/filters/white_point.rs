//! White point filter implementation.

use crate::FilterDerive;

/// Adjusts white balance using a target white point.
///
/// The provided RGB triplet defines the source white point to normalize.
///
/// # Parameters
///
/// - `red`: Red channel white-point component
/// - `green`: Green channel white-point component
/// - `blue`: Blue channel white-point component
#[derive(Debug, Clone, Copy, FilterDerive)]
#[filter(color_only, fragment = "fragments/white_point.wgsl")]
pub struct WhitePoint<R, G, B>(pub R, pub G, pub B);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn test_white_point_params() {
        let filter = WhitePoint(1.1f32, 1.0f32, 0.9f32);
        assert_eq!(filter.params(), [1.1, 1.0, 0.9]);
    }
}
