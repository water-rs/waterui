//! GTK ResolvedColor component implementation.

use gtk4::Widget;
use gtk4::prelude::*;
use waterui_core::{Environment, Native};
use waterui_graphics::color::{Color, ResolvedColor};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::{resolved_color_to_css_rgba, store_watcher_guard, subscribe_then_get};

fn color_widget() -> (gtk4::Box, gtk4::CssProvider) {
    let widget = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    widget.set_hexpand(true);
    widget.set_vexpand(true);
    widget.add_css_class("waterui-color");

    let provider = gtk4::CssProvider::new();
    widget
        .style_context()
        .add_provider(&provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);
    (widget, provider)
}

fn apply_color(provider: &gtk4::CssProvider, resolved: ResolvedColor) {
    provider.load_from_data(&format!(
        ".waterui-color {{ background-color: {}; }}",
        resolved_color_to_css_rgba(resolved)
    ));
}

impl GtkComponent for Native<Color> {
    fn render(self, env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let resolved = self.into_inner().resolve(env);
        let (widget, provider) = color_widget();
        let (initial, guard) = subscribe_then_get(&resolved, {
            let provider = provider.clone();
            move |context| {
                let provider = provider.clone();
                glib::idle_add_local_once(move || apply_color(&provider, context.into_value()));
            }
        });
        apply_color(&provider, initial);
        store_watcher_guard(&widget, Box::new(guard));
        widget.upcast()
    }
}

impl GtkComponent for Native<ResolvedColor> {
    fn render(self, _env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let resolved = self.into_inner();
        let (widget, provider) = color_widget();
        apply_color(&provider, resolved);
        widget.upcast()
    }
}
