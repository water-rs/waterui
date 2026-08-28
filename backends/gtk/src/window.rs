//! Window management utilities for GTK backend.

use gtk4::{Application, ApplicationWindow};
use nami::Signal;
use num_traits::ToPrimitive as _;
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

/// Offers "Inspect element" on a secondary click anywhere in the window.
///
/// A debug build attaches this to the window itself rather than to any widget,
/// so inspection is reachable from everywhere without an application opting in.
/// A widget with a context menu of its own handles the click first and this
/// never runs, which is the same precedence a browser gives page menus.
///
/// GTK builds native widgets and publishes no accessibility tree to the
/// inspector, so there is no node to name here; the inspector opens on the
/// application. Naming the element as well needs GTK to feed the tree channel.
pub fn install_inspect_gesture(window: &ApplicationWindow, env: &Environment) {
    use gtk4::prelude::*;

    if !cfg!(debug_assertions) {
        return;
    }
    // Nothing is listening in a release build, and an entry that does nothing is
    // worse than no entry.
    if env.get::<waterui::inspector::InspectorRuntime>().is_none() {
        return;
    }

    let click = gtk4::GestureClick::new();
    click.set_button(3);
    // Run after the target widget has had its turn, so an application's own
    // context menu wins.
    click.set_propagation_phase(gtk4::PropagationPhase::Bubble);
    click.connect_pressed({
        let window = window.clone();
        let env = env.clone();
        move |_, _, x, y| {
            if env.get::<waterui::inspector::InspectorRuntime>().is_none() {
                return;
            }
            let popover = gtk4::Popover::new();
            popover.set_has_arrow(true);
            popover.set_parent(&window);
            popover.set_pointing_to(Some(&gdk4::Rectangle::new(
                pointer_coordinate(x),
                pointer_coordinate(y),
                1,
                1,
            )));

            let item = gtk4::Button::with_label("Inspect element");
            item.set_has_frame(false);
            item.connect_clicked({
                let popover = popover.clone();
                // `Environment::get` hands back a reference borrowed from the
                // environment, which cannot escape into this `'static` handler.
                // The environment itself is cheap to clone, so the handler
                // carries one and resolves the runtime when the item is
                // actually clicked.
                let env = env.clone();
                move |_| {
                    popover.popdown();
                    let Some(inspector) = env.get::<waterui::inspector::InspectorRuntime>() else {
                        return;
                    };
                    inspector.open();
                }
            });
            popover.set_child(Some(&item));
            popover.popup();
        }
    });
    window.add_controller(click);
}

/// Rounds a GTK gesture coordinate to the integer pixel `GdkRectangle` takes.
///
/// GTK reports gesture positions as finite widget-local logical pixels, so the
/// rounded value is always well inside `i32`; anything else means GTK handed us
/// a coordinate for a window that cannot exist, which is worth crashing on
/// rather than silently pointing the popover somewhere else.
fn pointer_coordinate(value: f64) -> i32 {
    value
        .round()
        .to_i32()
        .unwrap_or_else(|| panic!("GTK pointer coordinate {value} does not fit i32"))
}

/// Applies `WaterUI` window background styling to a GTK window.
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
                let css = css;
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
