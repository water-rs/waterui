//! The window's two inset regions, from what `UIKit` reports.
//!
//! The container region is the view's safe area — notch, home indicator,
//! status bar, window chrome. The keyboard region is the keyboard layout
//! guide's overlap with the view. They are separate regions: layout resolves
//! the deeper of the two on each edge.

use cocoa_ui::geometry::EdgeInsets as PhysicalInsets;
use waterui_core::layout::LayoutDirection;
use waterui_layout::padding::EdgeInsets;

use crate::num_cast::f64_as_f32;

/// The container region for a safe area `UIKit` reports in points. The
/// physical left and right edges resolve to leading and trailing under
/// `direction`.
pub(super) const fn container(safe_area: PhysicalInsets, direction: LayoutDirection) -> EdgeInsets {
    let (leading, trailing) = if direction.is_right_to_left() {
        (safe_area.right, safe_area.left)
    } else {
        (safe_area.left, safe_area.right)
    };
    EdgeInsets::new(
        f64_as_f32(safe_area.top),
        f64_as_f32(safe_area.bottom),
        f64_as_f32(leading),
        f64_as_f32(trailing),
    )
}

/// The keyboard region for a view `height` points tall whose keyboard
/// guide's top edge sits `guide_top` points from the view's top.
///
/// With no keyboard the guide rests at the bottom of the safe area
/// (`safe_bottom` above the view's bottom edge), which is the container
/// region's business, not the keyboard's: only a guide above that rest line
/// is a keyboard, and its band then reaches from the guide to the view's
/// bottom edge.
pub(super) const fn keyboard(height: f64, guide_top: f64, safe_bottom: f64) -> EdgeInsets {
    let rest = height - safe_bottom;
    let bottom = if guide_top < rest {
        (height - guide_top).max(0.0)
    } else {
        0.0
    };
    EdgeInsets::new(0.0, f64_as_f32(bottom), 0.0, 0.0)
}

#[cfg(test)]
mod tests {
    use cocoa_ui::geometry::EdgeInsets as PhysicalInsets;
    use waterui_core::layout::LayoutDirection;
    use waterui_layout::padding::EdgeInsets;

    #[test]
    fn the_physical_edges_resolve_to_leading_and_trailing_by_direction() {
        let safe = PhysicalInsets::new(47.0, 10.0, 34.0, 20.0);
        assert_eq!(
            super::container(safe, LayoutDirection::LeftToRight),
            EdgeInsets::new(47.0, 34.0, 10.0, 20.0)
        );
        assert_eq!(
            super::container(safe, LayoutDirection::RightToLeft),
            EdgeInsets::new(47.0, 34.0, 20.0, 10.0)
        );
    }

    #[test]
    fn a_guide_at_rest_is_no_keyboard() {
        assert_eq!(
            super::keyboard(800.0, 766.0, 34.0),
            EdgeInsets::new(0.0, 0.0, 0.0, 0.0)
        );
    }

    #[test]
    fn a_guide_above_the_rest_line_is_a_keyboard_from_the_view_bottom() {
        assert_eq!(
            super::keyboard(800.0, 500.0, 34.0),
            EdgeInsets::new(0.0, 300.0, 0.0, 0.0)
        );
    }
}
