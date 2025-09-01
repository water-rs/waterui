//! Layout implementation for GTK4 backend.

pub mod scroll;
pub mod stack;

pub use scroll::*;
pub use stack::*;

use gtk4::{Align, Widget, prelude::*};
use waterui::component::layout::{Alignment, Edge, Frame};

/// Apply WaterUI frame configuration to a GTK4 widget.
pub fn apply_frame(widget: &Widget, frame: &Frame) {
    // Apply size constraints
    if !frame.width.is_nan() {
        widget.set_width_request(frame.width as i32);
    }
    if !frame.height.is_nan() {
        widget.set_height_request(frame.height as i32);
    }

    // Apply alignment
    let (halign, valign) = alignment_to_gtk(&frame.alignment);
    widget.set_halign(halign);
    widget.set_valign(valign);

    // Apply margins
    apply_margins(widget, &frame.margin);
}

/// Convert WaterUI alignment to GTK4 alignment.
fn alignment_to_gtk(alignment: &Alignment) -> (Align, Align) {
    match alignment {
        Alignment::Default => (Align::Fill, Align::Fill),
        Alignment::Leading => (Align::Start, Align::Start),
        Alignment::Center => (Align::Center, Align::Center),
        Alignment::Trailing => (Align::End, Align::End),
    }
}

/// Apply WaterUI margins to a GTK4 widget.
fn apply_margins(widget: &Widget, margin: &Edge) {
    if !margin.top.is_nan() {
        widget.set_margin_top(margin.top as i32);
    }
    if !margin.bottom.is_nan() {
        widget.set_margin_bottom(margin.bottom as i32);
    }
    if !margin.left.is_nan() {
        widget.set_margin_start(margin.left as i32);
    }
    if !margin.right.is_nan() {
        widget.set_margin_end(margin.right as i32);
    }
}
