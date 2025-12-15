//! GTK4 ScrollView component implementation.

use gtk4::prelude::*;
use gtk4::Widget;
use waterui_core::{Environment, Native};
use waterui_layout::scroll::{Axis, ScrollView};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;

impl GtkComponent for Native<ScrollView> {
    /// Renders a `WaterUI` `ScrollView` as a GTK4 `ScrolledWindow`.
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let (axis, content) = self.into_inner().into_inner();

        // Create the ScrolledWindow
        let scrolled_window = gtk4::ScrolledWindow::new();
        scrolled_window.set_hexpand(true);
        scrolled_window.set_vexpand(true);

        // Set scrollbar policies based on axis
        let (h_policy, v_policy) = match axis {
            Axis::Horizontal => (gtk4::PolicyType::Automatic, gtk4::PolicyType::Never),
            Axis::Vertical => (gtk4::PolicyType::Never, gtk4::PolicyType::Automatic),
            Axis::All => (gtk4::PolicyType::Automatic, gtk4::PolicyType::Automatic),
        };

        scrolled_window.set_policy(h_policy, v_policy);

        // Render and add the content
        let content_widget = renderer.render_any(content, env);
        scrolled_window.set_child(Some(&content_widget));

        scrolled_window.upcast()
    }
}
