//! Vibrance filter implementation.

use crate::FilterDerive;

/// Adjusts vibrance, boosting muted colors more than already saturated ones.
///
/// # Parameters
///
/// - `amount`: Vibrance amount (-1.0 = muted, 0.0 = unchanged, 1.0 = strongly boosted)
#[derive(Debug, Clone, Copy, FilterDerive)]
#[filter(color_only, fragment = "color/transform/vibrance.wgsl")]
pub struct Vibrance<T>(pub T);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn test_vibrance_params() {
        let filter = Vibrance(0.6f32);
        assert_eq!(filter.params(), [0.6]);
    }
}
