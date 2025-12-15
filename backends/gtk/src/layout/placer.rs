//! Layout placement utilities for GTK widgets.
//!
//! This module applies layout results from `waterui-layout` to GTK widgets
//! using `gtk4::Fixed` for absolute positioning.

use gtk4::prelude::*;
use gtk4::{Fixed, Widget};
use waterui_core::layout::Rect;

/// Places children in a `Fixed` container according to layout results.
///
/// # Arguments
///
/// * `container` - The `Fixed` container to place children in
/// * `rects` - Layout results from `waterui-layout`
/// * `children` - Child widgets to place
///
/// # Panics
///
/// Panics if `rects` and `children` have different lengths.
pub fn place_children(container: &Fixed, rects: &[Rect], children: &[Widget]) {
    assert_eq!(
        rects.len(),
        children.len(),
        "rects and children must have the same length"
    );

    for (rect, widget) in rects.iter().zip(children) {
        // Remove from current parent if necessary
        if let Some(parent) = widget.parent() {
            if let Some(fixed) = parent.downcast_ref::<Fixed>() {
                fixed.remove(widget);
            }
        }

        // Set size request
        widget.set_size_request(rect.width() as i32, rect.height() as i32);

        // Place in container at position
        container.put(widget, rect.x() as f64, rect.y() as f64);
    }
}

/// Updates the positions of already-placed children.
///
/// This is more efficient than `place_children` when children are already
/// in the container and only positions have changed.
pub fn update_positions(container: &Fixed, rects: &[Rect], children: &[Widget]) {
    assert_eq!(
        rects.len(),
        children.len(),
        "rects and children must have the same length"
    );

    for (rect, widget) in rects.iter().zip(children) {
        // Update size
        widget.set_size_request(rect.width() as i32, rect.height() as i32);

        // Move to new position
        container.move_(widget, rect.x() as f64, rect.y() as f64);
    }
}

/// Creates a `Fixed` container for layout.
#[must_use]
pub fn create_layout_container() -> Fixed {
    Fixed::new()
}
