//! Brightness filter implementation.

use crate::{Filter, FilterParam, SignalVisitor, StageCollector};

/// Adjusts the brightness of an image.
///
/// Adds the specified amount to each RGB channel.
///
/// # Parameters
///
/// - `amount`: Brightness adjustment (-1.0 = black, 0.0 = unchanged, 1.0 = white)
///
/// # Example
///
/// ```ignore
/// use filtrate::filters::Brightness;
///
/// // Static brightness
/// let bright = Brightness(0.2);
///
/// // Animated brightness
/// let amount = binding.computed();
/// let animated = Brightness(amount);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Brightness<T>(pub T);

impl<T: FilterParam> Filter for Brightness<T> {
    const COLOR_ONLY: bool = true;

    type Params = [f32; 1];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 1] {
        [self.0.snapshot()]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/fragments/brightness.wgsl")
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
    fn test_brightness_params() {
        let filter = Brightness(0.5f32);
        assert_eq!(filter.params(), [0.5]);
    }

    #[test]
    fn test_brightness_color_only() {
        assert!(Brightness::<f32>::COLOR_ONLY);
    }
}
