//! GTK ResolvedColor component implementation.

use gtk4::Widget;
use gtk4::prelude::*;
use waterui_core::{Environment, Native};
use waterui_graphics::color::{Color, ResolvedColor};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::{ScopedCss, resolved_color_to_css_rgba, store_watcher_guard, subscribe_then_get};

fn color_widget() -> (gtk4::Box, ScopedCss) {
    let widget = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    widget.set_hexpand(true);
    widget.set_vexpand(true);

    let css = ScopedCss::attach(
        &widget,
        "waterui-color",
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    (widget, css)
}

fn apply_color(css: &ScopedCss, resolved: ResolvedColor) {
    css.set_declarations(&format!(
        "background-color: {};",
        resolved_color_to_css_rgba(resolved)
    ));
}

impl GtkComponent for Native<Color> {
    fn render(self, env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let resolved = self.into_inner().resolve(env);
        let (widget, css) = color_widget();
        let (initial, guard) = subscribe_then_get(&resolved, {
            let css = css.clone();
            move |context| {
                let css = css.clone();
                glib::idle_add_local_once(move || apply_color(&css, context.into_value()));
            }
        });
        apply_color(&css, initial);
        store_watcher_guard(&widget, Box::new(guard));
        widget.upcast()
    }
}

impl GtkComponent for Native<ResolvedColor> {
    fn render(self, _env: &Environment, _renderer: &mut GtkRenderer) -> Widget {
        let resolved = self.into_inner();
        let (widget, css) = color_widget();
        apply_color(&css, resolved);
        widget.upcast()
    }
}
