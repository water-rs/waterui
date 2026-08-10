//! GTK4 Divider (Separator) component implementation.

use gtk4::Widget;
use gtk4::prelude::*;
use waterui::prelude::Divider;
use waterui_core::Environment;

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;

impl GtkComponent for Divider {
    /// Renders a `WaterUI` Divider component as a GTK4 Separator.
    ///
    /// The orientation is determined by the parent container's axis
    /// (stored in the environment).
    fn render(self, env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        // Get the axis from the environment to determine orientation
        // In a VStack, divider is horizontal; in HStack, divider is vertical
        let axis = env.get::<waterui_layout::stack::Axis>();

        // A vertical stack is separated by a horizontal rule, and vice versa;
        // with no axis in the environment we default to a horizontal rule.
        let orientation = match axis {
            Some(waterui_layout::stack::Axis::Vertical) | None => gtk4::Orientation::Horizontal,
            Some(_) => gtk4::Orientation::Vertical,
        };

        let separator = gtk4::Separator::new(orientation);

        // Expand to fill available space in the cross axis
        match orientation {
            gtk4::Orientation::Horizontal => separator.set_hexpand(true),
            gtk4::Orientation::Vertical => separator.set_vexpand(true),
            _ => {}
        }

        separator.upcast()
    }
}
