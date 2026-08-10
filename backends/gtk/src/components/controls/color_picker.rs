//! GTK4 `ColorPicker` component implementation.

use gtk4::prelude::*;
use gtk4::{ColorDialog, ColorDialogButton, Widget, gdk};
use nami::Signal;
use waterui_core::{Environment, Native};
use waterui_form::picker::color::ColorPickerConfig;
use waterui_graphics::color::Color;

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::store_watcher_guard;

impl GtkComponent for Native<ColorPickerConfig> {
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let config = self.into_inner();

        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        let label = renderer.render(config.label, env);
        root.append(&label);

        let dialog = ColorDialog::new();
        dialog.set_with_alpha(config.support_alpha);
        let button = ColorDialogButton::new(Some(dialog));

        if config.support_hdr {
            tracing::debug!(
                "GTK ColorPicker does not expose explicit HDR picker controls; using SDR UI"
            );
        }

        apply_color_button_rgba(&button, config.value.get(), env);

        let value = config.value.clone();
        button.connect_rgba_notify(move |button| {
            let rgba = button.rgba();
            value.set(rgba_to_color(rgba));
        });

        let guard = config.value.watch({
            let button = button.clone();
            let env = env.clone();
            move |ctx: nami::watcher::Context<Color>| {
                let color = ctx.into_value();
                let button = button.clone();
                let env = env.clone();
                glib::idle_add_local_once(move || {
                    apply_color_button_rgba(&button, color, &env);
                });
            }
        });

        root.append(&button);
        store_watcher_guard(&root, Box::new(guard));
        root.upcast()
    }
}

fn apply_color_button_rgba(button: &ColorDialogButton, color: Color, env: &Environment) {
    let resolved = color.resolve(env).get();
    let srgb = resolved.to_srgb_with_headroom();
    let rgba = gdk::RGBA::new(srgb.red, srgb.green, srgb.blue, resolved.opacity);
    button.set_rgba(&rgba);
}

fn rgba_to_color(rgba: gdk::RGBA) -> Color {
    Color::srgb_f32(rgba.red(), rgba.green(), rgba.blue()).with_opacity(rgba.alpha())
}
