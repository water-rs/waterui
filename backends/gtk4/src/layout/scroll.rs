//! Scroll view implementation for GTK4.

use gtk4::{Align, Box as GtkBox, Orientation, PolicyType, ScrolledWindow, Widget, prelude::*};
use waterui::{
    Environment,
    component::layout::scroll::{Axis, ScrollView},
};

use crate::renderer::render;

/// Render a scroll view using GTK4 ScrolledWindow.
pub fn render_scroll_view(scroll: ScrollView, env: &Environment) -> Widget {
    let scrolled_window = ScrolledWindow::new();
    
    // Scroll view should fill available space like SwiftUI
    scrolled_window.set_halign(Align::Fill);
    scrolled_window.set_valign(Align::Fill);
    scrolled_window.set_hexpand(true);
    scrolled_window.set_vexpand(true);

    // Set scroll policies
    let h_policy = match scroll.axis {
        Axis::Horizontal => PolicyType::Automatic,
        Axis::Vertical => PolicyType::Never,
        Axis::All => PolicyType::Automatic,
    };

    let v_policy = match scroll.axis {
        Axis::Horizontal => PolicyType::Never,
        Axis::Vertical => PolicyType::Automatic,
        Axis::All => PolicyType::Automatic,
    };

    scrolled_window.set_policy(h_policy, v_policy);

    // Add content wrapped in a centering container
    let content_widget = render(scroll.content, env);
    
    // Create a centering wrapper for the content
    let wrapper = GtkBox::new(Orientation::Vertical, 0);
    wrapper.set_halign(Align::Fill);
    wrapper.set_valign(Align::Fill);
    wrapper.set_hexpand(true);
    wrapper.set_vexpand(true);
    
    // Add the content centered within the wrapper
    content_widget.set_halign(Align::Center);
    content_widget.set_valign(Align::Center);
    wrapper.append(&content_widget);
    
    scrolled_window.set_child(Some(&wrapper));

    scrolled_window.upcast()
}
