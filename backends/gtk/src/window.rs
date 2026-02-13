//! Window management utilities for GTK backend.

use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow};
use nami::Signal;
use waterui::window::WindowBackground;
use waterui_core::Environment;
use waterui_core::resolve::Resolvable;
use waterui_graphics::color::ResolvedColor;

use crate::util::store_watcher_guard;

/// Creates a new application window with the specified properties.
#[must_use]
pub fn create_window(app: &Application, title: &str, width: i32, height: i32) -> ApplicationWindow {
    ApplicationWindow::builder()
        .application(app)
        .title(title)
        .default_width(width)
        .default_height(height)
        .build()
}

/// Applies WaterUI window background styling to a GTK window.
pub fn apply_window_background(
    window: &ApplicationWindow,
    background: &WindowBackground,
    env: &Environment,
) {
    match background {
        WindowBackground::Opaque => {}
        WindowBackground::Color(color) => {
            let provider = gtk4::CssProvider::new();
            window
                .style_context()
                .add_provider(&provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);
            window.add_css_class("waterui-window-background");

            let signal = color.resolve(env);

            // Initial apply
            apply_background_css(&provider, signal.get());

            // Reactive updates
            let guard = signal.watch({
                let provider = provider.clone();
                move |ctx| {
                    let resolved = ctx.into_value();
                    let provider = provider.clone();
                    glib::idle_add_local_once(move || apply_background_css(&provider, resolved));
                }
            });

            store_watcher_guard(window, guard);
        }
    }
}

fn apply_background_css(provider: &gtk4::CssProvider, resolved: ResolvedColor) {
    let srgb = resolved.to_srgb_with_headroom();
    let css = format!(
        ".waterui-window-background {{ background-color: rgba({}, {}, {}, {}); }}",
        (srgb.red.clamp(0.0, 1.0) * 255.0) as u8,
        (srgb.green.clamp(0.0, 1.0) * 255.0) as u8,
        (srgb.blue.clamp(0.0, 1.0) * 255.0) as u8,
        resolved.opacity.clamp(0.0, 1.0)
    );
    provider.load_from_data(&css);
}
