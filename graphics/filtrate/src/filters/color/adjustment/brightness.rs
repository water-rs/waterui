//! Brightness filter implementation.

use crate::Filter;

/// Adjusts the brightness of an image.
///
/// Adds the specified amount to each straight-alpha RGB channel. On
/// premultiplied colour that is `rgb + amount * a`, a linear map, so the
/// filter is [`LINEAR`](crate::ColorFilter::LINEAR). It carries a SIMD CPU
/// kernel.
///
/// # Parameters
///
/// - `amount`: Brightness adjustment (-1.0 = black, 0.0 = unchanged, 1.0 = white)
///
/// # Example
///
/// ```rust
/// # use filtrate::Filter;
/// use filtrate::filters::Brightness;
///
/// // Static brightness
/// let bright = Brightness(0.2_f32);
/// # assert_eq!(bright.params(), [0.2]);
/// ```
#[derive(Debug, Clone, Copy, Filter)]
#[filter(
    color,
    shader = "color/adjustment/brightness.wgsl",
    linear = true,
    cpu = crate::cpu::brightness
)]
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
    fn brightness_is_a_linear_colour_filter() {
        const { assert!(<Brightness<f32> as crate::ColorFilter>::LINEAR) };
    }
}
