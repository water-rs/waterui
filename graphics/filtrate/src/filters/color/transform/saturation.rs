//! Saturation filter implementation.

use crate::Filter;

/// Adjusts the color saturation of an image.
///
/// Mixes between the working-space luma and the original colour, a linear
/// map, so the filter is [`LINEAR`](crate::ColorFilter::LINEAR). It carries
/// a SIMD CPU kernel.
///
/// # Parameters
///
/// - `amount`: Saturation multiplier (0.0 = grayscale, 1.0 = unchanged, >1.0 = more saturated)
///
/// # Example
///
/// ```rust
/// # use filtrate::Filter;
/// use filtrate::filters::Saturation;
///
/// let desaturated = Saturation(0.5_f32);
/// let vibrant = Saturation(1.5_f32);
/// # assert_eq!(desaturated.params(), [0.5]);
/// # assert_eq!(vibrant.params(), [1.5]);
/// ```
#[derive(Debug, Clone, Copy, Filter)]
#[filter(
    color,
    shader = "color/transform/saturation.wgsl",
    linear = true,
    cpu = crate::cpu::saturation
)]
pub struct Saturation<T>(pub T);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn test_saturation_params() {
        let filter = Saturation(0.5f32);
        assert_eq!(filter.params(), [0.5]);
    }
}
