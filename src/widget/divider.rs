//! Divider widget shared across `WaterUI` surfaces.

#[cfg(feature = "menu")]
use alloc::{vec, vec::Vec};
#[cfg(feature = "menu")]
use nami::Computed;
#[cfg(feature = "menu")]
use waterui_controls::menu::{MenuItem, MenuView};
use waterui_core::layout::StretchAxis;
use waterui_core::raw_view;

/// A thin line that separates content.
///
/// Divider adapts to its parent container: in `VStack` it spans horizontally,
/// in `HStack` it spans vertically. The line is drawn in the theme's separator
/// colour — the [`Border`](crate::theme::color::Border) slot — so it follows
/// the active theme and colour scheme on every backend.
#[derive(Debug, Clone, Copy)]
#[must_use]
pub struct Divider;

// A divider always spans its parent stack's cross axis and stays one point
// thick on the main axis — which orientation that is resolves from
// `stack::Axis`, but the cross-axis answer is the same either way.
raw_view!(Divider, StretchAxis::CrossAxis);

#[cfg(feature = "menu")]
impl From<Divider> for MenuItem {
    fn from(_: Divider) -> Self {
        Self::Divider
    }
}

#[cfg(feature = "menu")]
impl MenuView for Divider {
    fn into_menu_items(self) -> Computed<Vec<MenuItem>> {
        Computed::constant(vec![MenuItem::Divider])
    }
}

#[cfg(all(test, feature = "menu"))]
mod tests {
    use nami::Signal as _;
    use waterui_controls::menu::{MenuItem, MenuView as _};

    use super::Divider;

    #[test]
    fn divider_converts_to_menu_item() {
        assert!(matches!(MenuItem::from(Divider), MenuItem::Divider));
        let items = Divider.into_menu_items().snapshot();
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], MenuItem::Divider));
    }
}
