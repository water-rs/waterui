//! Layout placement utilities for GTK widgets.
//!
//! This module applies layout results from `waterui-layout` to GTK widgets
//! using `gtk4::Fixed` for absolute positioning.

use gtk4::prelude::*;
use gtk4::{Fixed, Widget};
use waterui_core::layout::{Rect, StretchAxis};

fn layout_debug_enabled() -> bool {
    std::env::var_os("WATERUI_GTK_LAYOUT_DEBUG").is_some()
}

fn resolve_size_request(widget: &Widget, axis: StretchAxis, req_w: i32, req_h: i32) -> (i32, i32) {
    let width_request = req_w;
    let type_name = widget.type_().name();
    let is_layout_container = type_name == "WuiFixedContainer";
    let is_gl_area = type_name == "GtkGLArea" || widget.is::<gtk4::GLArea>();
    // Content-sized widgets should keep intrinsic height, but layout-driven
    // containers / GPU surfaces must receive explicit height to avoid 0x0 allocations.
    let keep_layout_height = axis.stretches_vertical() || is_layout_container || is_gl_area;
    let height_request = if keep_layout_height { req_h } else { -1 };
    (width_request, height_request)
}

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
#[allow(
    clippy::cast_possible_truncation,
    reason = "GTK widget geometry is integer pixels while WaterUI layout is f32"
)]
pub fn place_children(container: &Fixed, rects: &[Rect], children: &[(Widget, StretchAxis)]) {
    assert_eq!(
        rects.len(),
        children.len(),
        "rects and children must have the same length"
    );

    for (rect, (widget, axis)) in rects.iter().zip(children) {
        let margin_h = widget.margin_start() + widget.margin_end();
        let margin_v = widget.margin_top() + widget.margin_bottom();
        let req_w = (rect.width().round() as i32 - margin_h).max(0);
        let req_h = (rect.height().round() as i32 - margin_v).max(0);
        let (width_request, height_request) = resolve_size_request(widget, *axis, req_w, req_h);
        if layout_debug_enabled() {
            tracing::debug!(
                target: "waterui::gtk::layout",
                widget_type = %widget.type_().name(),
                axis = ?axis,
                requested_width = req_w,
                requested_height = req_h,
                applied_width = width_request,
                applied_height = height_request,
                x = rect.x(),
                y = rect.y(),
                width = rect.width(),
                height = rect.height(),
                "Placed GTK layout child"
            );
        }
        let mut already_in_container = false;

        // Remove from current parent if necessary
        if let Some(parent) = widget.parent()
            && let Some(fixed) = parent.downcast_ref::<Fixed>()
        {
            if fixed.as_ptr() == container.as_ptr() {
                already_in_container = true;
            } else {
                fixed.remove(widget);
            }
        }

        // Set size request
        widget.set_size_request(width_request, height_request);

        if already_in_container {
            container.move_(widget, f64::from(rect.x()), f64::from(rect.y()));
        } else {
            container.put(widget, f64::from(rect.x()), f64::from(rect.y()));
        }
    }
}

/// Updates the positions of already-placed children.
///
/// This is more efficient than `place_children` when children are already
/// in the container and only positions have changed.
///
/// # Panics
///
/// Panics if `rects` and `children` have different lengths.
#[allow(
    clippy::cast_possible_truncation,
    reason = "GTK widget geometry is integer pixels while WaterUI layout is f32"
)]
pub fn update_positions(container: &Fixed, rects: &[Rect], children: &[(Widget, StretchAxis)]) {
    assert_eq!(
        rects.len(),
        children.len(),
        "rects and children must have the same length"
    );

    for (rect, (widget, axis)) in rects.iter().zip(children) {
        let margin_h = widget.margin_start() + widget.margin_end();
        let margin_v = widget.margin_top() + widget.margin_bottom();
        let req_w = (rect.width().round() as i32 - margin_h).max(0);
        let req_h = (rect.height().round() as i32 - margin_v).max(0);
        let (width_request, height_request) = resolve_size_request(widget, *axis, req_w, req_h);
        if layout_debug_enabled() {
            tracing::debug!(
                target: "waterui::gtk::layout",
                widget_type = %widget.type_().name(),
                axis = ?axis,
                requested_width = req_w,
                requested_height = req_h,
                applied_width = width_request,
                applied_height = height_request,
                x = rect.x(),
                y = rect.y(),
                width = rect.width(),
                height = rect.height(),
                "Updated GTK layout child position"
            );
        }

        // Update size
        widget.set_size_request(width_request, height_request);

        // Move to new position
        container.move_(widget, f64::from(rect.x()), f64::from(rect.y()));
    }
}

/// Creates a `Fixed` container for layout.
#[must_use]
pub fn create_layout_container() -> Fixed {
    Fixed::new()
}
