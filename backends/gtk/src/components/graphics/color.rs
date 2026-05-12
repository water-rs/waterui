//! GTK ResolvedColor component implementation.

use gtk4::Widget;
use gtk4::prelude::*;
use waterui_core::{Environment, Native};
use waterui_graphics::color::ResolvedColor;

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::resolved_color_to_css_rgba;

impl GtkComponent for Native<ResolvedColor> {
    fn render(self, _env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let resolved = self.into_inner();

        let widget = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        widget.set_hexpand(true);
        widget.set_vexpand(true);

        widget.add_css_class("waterui-resolved-color");
        let provider = gtk4::CssProvider::new();
        widget
            .style_context()
            .add_provider(&provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);

        let css = format!(
            ".waterui-resolved-color {{ background-color: {}; }}",
            resolved_color_to_css_rgba(resolved)
        );
        provider.load_from_data(&css);

        widget.upcast()
    }
}
