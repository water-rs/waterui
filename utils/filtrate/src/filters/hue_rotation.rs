//! Hue rotation filter implementation.

use crate::{Filter, FilterParam, SignalVisitor, StageCollector};

/// Rotates the hue of all colors around the color wheel.
///
/// Converts to HSL, rotates hue, and converts back to RGB.
///
/// # Parameters
///
/// - `angle`: Rotation angle in degrees (0-360)
///
/// # Example
///
/// ```ignore
/// use filtrate::filters::HueRotation;
///
/// // Rotate 180 degrees (complementary colors)
/// let complement = HueRotation(180.0);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct HueRotation<T>(pub T);

impl<T: FilterParam> Filter for HueRotation<T> {
    const COLOR_ONLY: bool = true;

    type Params = [f32; 1];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 1] {
        [self.0.snapshot()]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/fragments/hue_rotation.wgsl")
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
    fn test_hue_rotation_params() {
        let filter = HueRotation(90.0f32);
        assert_eq!(filter.params(), [90.0]);
    }
}
