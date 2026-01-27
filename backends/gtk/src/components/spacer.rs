//! GTK Spacer component implementation.

use gtk4::Widget;
use gtk4::prelude::*;
use waterui_core::{Environment, Native, NativeView};
use waterui_layout::spacer::Spacer;

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;

impl GtkComponent for Native<Spacer> {
    /// Renders a `WaterUI` Spacer as an empty GTK widget that expands.
    ///
    /// The spacer is implemented as an empty `Box` widget with expansion
    /// properties set based on the spacer's axis configuration.
    fn render(self, _env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let spacer = self.into_inner();

        let widget = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);

        // Set expand based on stretch axis
        let stretch = spacer.stretch_axis();
        widget.set_hexpand(stretch.stretches_horizontal());
        widget.set_vexpand(stretch.stretches_vertical());

        widget.upcast()
    }
}
