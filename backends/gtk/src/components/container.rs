//! GTK container component using native GTK layout.

use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Orientation, Widget};
use waterui_core::{Environment, Native};
use waterui_layout::container::FixedContainer;
use waterui_layout::StretchAxis;

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;

impl GtkComponent for Native<FixedContainer> {
    /// Renders a `FixedContainer` to a GTK `Box` widget.
    ///
    /// This uses GTK's native Box layout instead of waterui-layout
    /// for better integration with GTK's sizing system.
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let (layout, children) = self.into_inner().into_inner();

        // Infer orientation from stretch_axis:
        // - VStack stretches Horizontal (fills width) -> children are vertical
        // - HStack stretches Vertical (fills height) -> children are horizontal
        // - ZStack stretches Both -> use vertical as default
        let orientation = match layout.stretch_axis() {
            StretchAxis::Vertical => Orientation::Horizontal,
            // Horizontal, Both, and None all default to vertical orientation
            _ => Orientation::Vertical,
        };

        // Create the Box container with default spacing
        let container = GtkBox::new(orientation, 8);

        // Render each child view and add to container
        for view in children {
            let widget = renderer.render_any(view, env);
            container.append(&widget);
        }

        container.upcast()
    }
}
