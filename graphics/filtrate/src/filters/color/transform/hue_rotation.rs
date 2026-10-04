//! Hue rotation filter implementation.

use crate::Filter;

/// Rotates the hue of all colors around the color wheel.
///
/// Applies the CSS/SVG `hue-rotate` matrix (Filter Effects Module Level 1,
/// `feColorMatrix type="hueRotate"`) to the premultiplied colour, alpha
/// unchanged — a linear map, so the filter is
/// [`LINEAR`](crate::ColorFilter::LINEAR). It carries a SIMD CPU kernel.
///
/// # Parameters
///
/// - `angle`: Rotation angle in degrees (0-360)
///
/// # Example
///
/// ```rust
/// # use filtrate::Filter;
/// use filtrate::filters::HueRotation;
///
/// // Rotate 180 degrees (complementary colors)
/// let complement = HueRotation(180.0_f32);
/// # assert_eq!(complement.params(), [180.0]);
/// ```
#[derive(Debug, Clone, Copy, Filter)]
#[filter(
    color,
    shader = "color/transform/hue_rotation.wgsl",
    linear = true,
    cpu = crate::cpu::hue_rotation
)]
pub struct HueRotation<T>(pub T);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn test_hue_rotation_params() {
        let filter = HueRotation(90.0f32);
        assert_eq!(filter.params(), [90.0]);
    }
}
