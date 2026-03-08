//! Vibrance filter implementation.

use crate::Filter;
use nami::Signal;

/// Adjusts vibrance, boosting muted colors more than already saturated ones.
///
/// # Parameters
///
/// - `amount`: Vibrance amount (-1.0 = muted, 0.0 = unchanged, 1.0 = strongly boosted)
#[derive(Debug, Clone, Copy)]
pub struct Vibrance<T>(pub T);

impl<T: Signal<Output = f32> + 'static> Filter for Vibrance<T> {
    const COLOR_ONLY: bool = true;

    type Params = [f32; 1];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 1] {
        [self.0.get()]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/fragments/vibrance.wgsl")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vibrance_params() {
        let filter = Vibrance(0.6f32);
        assert_eq!(filter.params(), [0.6]);
    }
}
