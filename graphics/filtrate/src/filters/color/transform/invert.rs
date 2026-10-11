//! Invert filter implementation.

use crate::Filter;

/// Inverts all colors in an image.
///
/// Applies `1.0 - color` to each straight-alpha RGB channel, preserving
/// alpha. On premultiplied colour that is `a - rgb`, a linear map, so the
/// filter is [`LINEAR`](crate::ColorFilter::LINEAR).
///
/// # Example
///
/// ```rust
/// # use filtrate::Filter;
/// use filtrate::filters::Invert;
///
/// let inverted = Invert;
/// # assert_eq!(inverted.params().len(), 0);
/// ```
#[derive(Debug, Clone, Copy, Default, Filter)]
#[filter(color, shader = "color/transform/invert.wgsl", linear = true)]
pub struct Invert;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn test_invert_no_params() {
        let filter = Invert;
        assert_eq!(filter.params().len(), 0);
    }

    #[test]
    fn invert_is_a_linear_colour_filter() {
        const { assert!(<Invert as crate::ColorFilter>::LINEAR) };
    }
}
