use gtk4::{Align, Box as GtkBox, Orientation, Widget, prelude::*};
use waterui::{
    AnyView, Environment,
    component::layout::stack::{Stack, StackMode},
};

use crate::renderer::render;

/// Check if a widget should expand horizontally in a VStack (like SwiftUI behavior)
fn is_expandable_widget(widget: &Widget) -> bool {
    // Check widget type name to determine if it should expand
    let type_name = widget.type_().name();
    let type_str = type_name.to_string();

    matches!(
        type_str.as_str(),
        "GtkScale" |        // Slider
        "GtkEntry" |        // TextField  
        "GtkBox" |          // Container that might contain form elements
        "GtkCheckButton" |  // Toggle/Checkbox
        "GtkButton" // Button
    )
}

/// Render a generic Stack based on its mode - takes ownership
/// This implementation uses SwiftUI-inspired layout principles within GTK4's constraints
pub fn render_stack(stack: Stack, env: &Environment) -> Widget {
    match stack.mode {
        StackMode::Vertical => render_vstack(stack.contents, env),
        StackMode::Horizonal => render_hstack(stack.contents, env),
        StackMode::Layered => render_zstack(stack.contents, env),
    }
}

/// Render a vertical stack with SwiftUI-like behavior
fn render_vstack(contents: Vec<AnyView>, env: &Environment) -> Widget {
    let outer_box = GtkBox::new(Orientation::Vertical, 0);

    // The outer container should expand to fill available space
    outer_box.set_hexpand(true);
    outer_box.set_vexpand(true);
    outer_box.set_halign(Align::Fill);
    outer_box.set_valign(Align::Fill);

    // Create an inner centered container for the actual content
    let inner_box = GtkBox::new(Orientation::Vertical, 8);
    inner_box.set_halign(Align::Center); // Center the content container
    inner_box.set_valign(Align::Center);
    inner_box.set_width_request(400); // Set a reasonable max width like SwiftUI forms
    inner_box.set_hexpand(false); // Don't expand, stay at preferred width

    // Add content to inner box
    for child in contents {
        let child_widget = render(child, env);
        // Form elements should expand horizontally, text should be centered
        if is_expandable_widget(&child_widget) {
            child_widget.set_halign(Align::Fill);
            child_widget.set_hexpand(true);
        } else {
            child_widget.set_halign(Align::Center);
        }
        inner_box.append(&child_widget);
    }

    outer_box.append(&inner_box);
    outer_box.upcast()
}

/// Render a horizontal stack with SwiftUI-like behavior
fn render_hstack(contents: Vec<AnyView>, env: &Environment) -> Widget {
    let outer_box = GtkBox::new(Orientation::Horizontal, 0);

    // The outer container should expand to fill available space
    outer_box.set_hexpand(true);
    outer_box.set_vexpand(true);
    outer_box.set_halign(Align::Fill);
    outer_box.set_valign(Align::Fill);

    // Create an inner centered container for the actual content
    let inner_box = GtkBox::new(Orientation::Horizontal, 8);
    inner_box.set_halign(Align::Center);
    inner_box.set_valign(Align::Center);

    // Add content to inner box
    for child in contents {
        let child_widget = render(child, env);
        child_widget.set_valign(Align::Center);
        inner_box.append(&child_widget);
    }

    outer_box.append(&inner_box);
    outer_box.upcast()
}

/// Render a layered stack with SwiftUI-like behavior
fn render_zstack(contents: Vec<AnyView>, env: &Environment) -> Widget {
    let overlay = gtk4::Overlay::new();

    // ZStack fills available space
    overlay.set_hexpand(true);
    overlay.set_vexpand(true);
    overlay.set_halign(Align::Fill);
    overlay.set_valign(Align::Fill);

    let mut first = true;
    for child in contents {
        let child_widget = render(child, env);
        // Center each child in the layered stack
        child_widget.set_halign(Align::Center);
        child_widget.set_valign(Align::Center);

        if first {
            overlay.set_child(Some(&child_widget));
            first = false;
        } else {
            overlay.add_overlay(&child_widget);
        }
    }

    overlay.upcast()
}
