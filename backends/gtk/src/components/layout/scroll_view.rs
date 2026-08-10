//! GTK4 `ScrollView` component implementation.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::Widget;
use gtk4::prelude::*;
use nami::Signal;
use waterui_core::layout::Point;
use waterui_core::{Environment, Native};
use waterui_layout::scroll::{Axis, ScrollView};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::store_watcher_guard;

fn apply_scroll_target(scrolled_window: &gtk4::ScrolledWindow, axis: Axis, target: Point) {
    let horizontal = scrolled_window.hadjustment();
    let vertical = scrolled_window.vadjustment();
    match axis {
        Axis::Horizontal => {
            horizontal.set_value(f64::from(target.x).clamp(
                horizontal.lower(),
                (horizontal.upper() - horizontal.page_size()).max(horizontal.lower()),
            ));
        }
        Axis::Vertical => {
            vertical.set_value(f64::from(target.y).clamp(
                vertical.lower(),
                (vertical.upper() - vertical.page_size()).max(vertical.lower()),
            ));
        }
        Axis::All => {
            horizontal.set_value(f64::from(target.x).clamp(
                horizontal.lower(),
                (horizontal.upper() - horizontal.page_size()).max(horizontal.lower()),
            ));
            vertical.set_value(f64::from(target.y).clamp(
                vertical.lower(),
                (vertical.upper() - vertical.page_size()).max(vertical.lower()),
            ));
        }
        _ => panic!("GTK backend does not support scroll axis {axis:?}"),
    }
}

impl GtkComponent for Native<ScrollView> {
    /// Renders a `WaterUI` `ScrollView` as a GTK4 `ScrolledWindow`.
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let (axis, content, controller) = self.into_inner().into_inner();

        // Create the ScrolledWindow
        let scrolled_window = gtk4::ScrolledWindow::new();
        scrolled_window.set_hexpand(true);
        scrolled_window.set_vexpand(true);

        // Reset any default margins to ensure it fills the window
        scrolled_window.set_margin_top(0);
        scrolled_window.set_margin_bottom(0);
        scrolled_window.set_margin_start(0);
        scrolled_window.set_margin_end(0);

        // Set scrollbar policies based on axis
        let (h_policy, v_policy) = match axis {
            Axis::Horizontal => (gtk4::PolicyType::Automatic, gtk4::PolicyType::Never),
            Axis::Vertical => (gtk4::PolicyType::Never, gtk4::PolicyType::Automatic),
            Axis::All | _ => (gtk4::PolicyType::Automatic, gtk4::PolicyType::Automatic),
        };

        scrolled_window.set_policy(h_policy, v_policy);

        // Render and add the content
        let content_widget = renderer.render_any(content, env);
        scrolled_window.set_child(Some(&content_widget));

        if let Some(controller) = controller {
            let requested = Rc::new(Cell::new(None::<Point>));
            for adjustment in [scrolled_window.hadjustment(), scrolled_window.vadjustment()] {
                let requested = Rc::clone(&requested);
                let scrolled_window = scrolled_window.clone();
                adjustment.connect_changed(move |_| {
                    if let Some(target) = requested.get() {
                        apply_scroll_target(&scrolled_window, axis, target);
                    }
                });
            }
            let generation = controller.generation();
            let target = controller.target();
            let scrolled_for_watch = scrolled_window.clone();
            let requested_for_watch = Rc::clone(&requested);
            let guard = generation.watch(move |_| {
                let target = target.get();
                requested_for_watch.set(Some(target));
                let scrolled_window = scrolled_for_watch.clone();
                glib::idle_add_local_once(move || {
                    apply_scroll_target(&scrolled_window, axis, target);
                });
            });
            store_watcher_guard(&scrolled_window, Box::new(guard));
        }

        scrolled_window.upcast()
    }
}
