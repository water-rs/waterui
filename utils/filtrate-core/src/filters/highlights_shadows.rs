//! Highlights/shadows filter implementation.

use crate::Filter;
use nami::Signal;

/// Lifts shadows and recovers highlights.
///
/// # Parameters
///
/// - `highlights`: Highlight recovery amount (-1.0 = brighter highlights, 1.0 = more recovery)
/// - `shadows`: Shadow lift amount (-1.0 = darker shadows, 1.0 = more lift)
#[derive(Debug, Clone, Copy)]
pub struct HighlightsShadows<H, S>(pub H, pub S);

impl<H: Signal<Output = f32> + 'static, S: Signal<Output = f32> + 'static> Filter
    for HighlightsShadows<H, S>
{
    const COLOR_ONLY: bool = true;

    type Params = [f32; 2];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 2] {
        [self.0.get(), self.1.get()]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/fragments/highlights_shadows.wgsl")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_highlights_shadows_params() {
        let filter = HighlightsShadows(0.4f32, 0.3f32);
        assert_eq!(filter.params(), [0.4, 0.3]);
    }
}
