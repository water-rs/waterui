//! Scroll view implementation for GTK4.

use gtk4::{PolicyType, ScrolledWindow, Widget, prelude::*};
use waterui::{
    Environment, View,
    component::layout::scroll::{Axis, ScrollView},
};

use crate::renderer::render;

/// Render a scroll view using GTK4 ScrolledWindow.
pub fn render_scroll_view<V: View>(scroll: ScrollView, env: &Environment) -> Widget {
    let scrolled_window = ScrolledWindow::new();

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

    // Add content
    let content_widget = render(scroll.content, env);
    scrolled_window.set_child(Some(&content_widget));

    scrolled_window.upcast()
}
