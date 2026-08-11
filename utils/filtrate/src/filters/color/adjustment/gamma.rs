//! Gamma filter implementation.

use crate::Filter;

/// Adjusts image gamma.
///
/// # Parameters
///
/// - `gamma`: Gamma exponent (>0.0, 1.0 = unchanged)
#[derive(Debug, Clone, Copy, Filter)]
#[filter(color_only, shader = "color/adjustment/gamma.wgsl")]
pub struct Gamma<T>(pub T);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn test_gamma_params() {
        let filter = Gamma(2.2f32);
        assert_eq!(filter.params(), [2.2]);
    }
}
