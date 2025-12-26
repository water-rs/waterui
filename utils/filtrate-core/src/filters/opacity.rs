//! Opacity filter implementation.

use crate::Filter;
use nami::Signal;

/// Adjusts the opacity (alpha transparency) of an image.
///
/// # Parameters
///
/// - `amount`: Opacity multiplier (0.0 = transparent, 1.0 = opaque)
///
/// # Example
///
/// ```ignore
/// use filtrate_core::filters::Opacity;
///
/// // 50% opacity
/// let faded = Opacity(0.5);
///
/// // Animated opacity for fade effects
/// let amount = binding.computed();
/// let animated = Opacity(amount);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Opacity<T>(pub T);

impl<T: Signal<Output = f32> + 'static> Filter for Opacity<T> {
    const COLOR_ONLY: bool = true;

    type Params = [f32; 1];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 1] {
        [self.0.get()]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/fragments/opacity.wgsl")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opacity_params() {
        let filter = Opacity(0.5f32);
        assert_eq!(filter.params(), [0.5]);
    }

    #[test]
    fn test_opacity_color_only() {
        assert!(Opacity::<f32>::COLOR_ONLY);
    }
}
