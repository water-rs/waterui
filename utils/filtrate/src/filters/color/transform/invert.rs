//! Invert filter implementation.

use crate::FilterDerive;

/// Inverts all colors in an image.
///
/// Applies `1.0 - color` to each RGB channel, preserving alpha.
///
/// # Example
///
/// ```ignore
/// use filtrate::filters::Invert;
///
/// let inverted = Invert;
/// ```
#[derive(Debug, Clone, Copy, Default, FilterDerive)]
#[filter(color_only, fragment = "color/transform/invert.wgsl")]
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
    fn test_invert_color_only() {
        assert!(Invert::COLOR_ONLY);
    }
}
