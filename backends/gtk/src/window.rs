//! Window management utilities for GTK backend.

use gtk4::{Application, ApplicationWindow};
use nami::Signal;
use waterui::window::WindowBackground;
use waterui_core::Environment;
use waterui_graphics::color::ResolvedColor;

use crate::util::{ScopedCss, resolved_color_to_css_rgba, store_watcher_guard};

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
            let css = ScopedCss::attach(
                window,
                "waterui-window-background",
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );

            let signal = color.resolve(env);

            // Initial apply
            apply_background_css(&css, signal.get());

            // Reactive updates
            let guard = signal.watch({
                let css = css.clone();
                move |ctx| {
                    let resolved = ctx.into_value();
                    let css = css.clone();
                    glib::idle_add_local_once(move || apply_background_css(&css, resolved));
                }
            });

            store_watcher_guard(window, guard);
        }
    }
}

fn apply_background_css(css: &ScopedCss, resolved: ResolvedColor) {
    css.set_declarations(&format!(
        "background-color: {};",
        resolved_color_to_css_rgba(resolved)
    ));
}
