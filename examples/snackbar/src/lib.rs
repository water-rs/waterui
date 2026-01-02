//! Snackbar test playground

use core::time::Duration;
use waterui::prelude::*;
use waterui::app::App;
use waterui::snackbar::{Snackbar, SnackbarManager, SnackbarPosition};
use waterui_icon::SystemIcon;

fn main() -> impl View {
    SnackbarDemo
}

struct SnackbarDemo;

impl View for SnackbarDemo {
    fn body(self, env: &Environment) -> impl View {
        let manager = env.get::<SnackbarManager>().expect("SnackbarManager not found");

        let manager1 = manager.clone();
        let manager2 = manager.clone();
        let manager3 = manager.clone();
        let manager4 = manager.clone();
        let manager5 = manager.clone();

        scroll(vstack((
            text("Snackbar Demo").size(24.0).bold(),
            spacer(),

            button("Simple Snackbar").action(move || {
                manager1.show(Snackbar::new("Hello from Snackbar!"));
            }),

            button("With Icon").action(move || {
                manager2.show(
                    Snackbar::new("File saved successfully")
                        .icon(SystemIcon::CHECKMARK),
                );
            }),

            button("With Action Button").action(move || {
                manager3.show(
                    Snackbar::new("Item moved to trash")
                        .icon(SystemIcon::TRASH)
                        .action("Undo", || {
                            waterui::log::info!("Undo clicked!");
                        })
                        .duration(Duration::from_secs(5)),
                );
            }),

            button("Top Position").action(move || {
                manager4.show(
                    Snackbar::new("Network connected")
                        .icon(SystemIcon::CHECKMARK)
                        .position(SnackbarPosition::TopCenter),
                );
            }),

            button("Queue Multiple").action(move || {
                manager5.show(Snackbar::new("First message"));
                manager5.show(Snackbar::new("Second message"));
                manager5.show(Snackbar::new("Third message"));
            }),

            spacer(),
        )).padding())
    }
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
