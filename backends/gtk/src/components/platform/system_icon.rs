//! GTK4 SystemIcon component implementation.

use gtk4::Widget;
use waterui_core::{Environment, Native};
use waterui_icon::SystemIcon;

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;

impl GtkComponent for Native<SystemIcon> {
    fn render(self, _env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let icon = self.into_inner();
        panic!(
            "SystemIcon `{}` is unsupported on GTK because Linux has no OS-supplied \
             cross-distribution icon catalog; use a packaged WaterUI icon crate",
            icon.name.as_str()
        )
    }
}
