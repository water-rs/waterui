//! Highlights/shadows filter implementation.

use crate::Filter;

/// Lifts shadows and recovers highlights.
///
/// # Parameters
///
/// - `highlights`: Highlight recovery amount (-1.0 = brighter highlights, 1.0 = more recovery)
/// - `shadows`: Shadow lift amount (-1.0 = darker shadows, 1.0 = more lift)
#[derive(Debug, Clone, Copy, Filter)]
#[filter(color_only, shader = "color/adjustment/highlights_shadows.wgsl")]
pub struct HighlightsShadows<H, S>(pub H, pub S);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn test_highlights_shadows_params() {
        let filter = HighlightsShadows(0.4f32, 0.3f32);
        assert_eq!(filter.params(), [0.4, 0.3]);
    }
}
