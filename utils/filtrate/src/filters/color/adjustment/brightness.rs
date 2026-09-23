//! Brightness filter implementation.

use crate::Filter;

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
/// ```
#[derive(Debug, Clone, Copy, Filter)]
#[filter(color_only, fragment = "color/adjustment/brightness.wgsl")]
pub struct Brightness<T>(pub T);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

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
