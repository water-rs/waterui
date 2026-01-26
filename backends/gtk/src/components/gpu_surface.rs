use gtk4::prelude::*;
use gtk4::Widget;
use waterui_core::{Environment, Native};
use waterui_graphics::gpu_surface::GpuSurface;

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;

impl GtkComponent for Native<GpuSurface> {
    fn render(self, _env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        // GPU-backed surfaces are optional in the GTK backend. Keep this as a
        // lightweight fallback so the backend compiles without GPU features.
        gtk4::Label::new(Some("GpuSurface not available")).upcast()
    }
}
