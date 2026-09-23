//! Trait for rendering `WaterUI` views as GTK widgets.

use gtk4::Widget;
use waterui_core::{Environment, View};

use crate::renderer::GtkRenderer;

/// A `WaterUI` view that can be rendered as a GTK widget.
///
/// This trait is implemented for view types (e.g., `Native<SliderConfig>`, `Divider`)
/// that map to GTK widgets.
pub trait GtkComponent: View {
    /// Renders this view as a GTK widget.
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget;
}
