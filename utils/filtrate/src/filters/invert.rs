//! Invert filter implementation.

use crate::{Filter, StageCollector};

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
#[derive(Debug, Clone, Copy, Default)]
pub struct Invert;

impl Filter for Invert {
    const COLOR_ONLY: bool = true;

    type Params = [f32; 0];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 0] {
        []
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/fragments/invert.wgsl")
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.color_fragment(self.fragments(), 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
