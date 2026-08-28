//! Conversion of kurbo rects into the accesskit rect type used when emitting
//! accessibility node bounds.

use super::*;

#[cfg(feature = "accessibility")]
pub(crate) fn kurbo_rect_to_accesskit_rect(rect: vello::kurbo::Rect) -> AccessibilityRect {
    AccessibilityRect {
        x0: rect.x0,
        y0: rect.y0,
        x1: rect.x1,
        y1: rect.y1,
    }
}
