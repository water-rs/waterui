//! Snackbar test playground

use core::time::Duration;
use waterui::app::App;
use waterui::prelude::*;
use waterui::preview;
use waterui::snackbar::{Snackbar, SnackbarManager, SnackbarPosition};
use waterui_icon::system_icon;

#[preview]
fn main() -> impl View {
    scroll(
        vstack((
            text("Snackbar Demo").title().bold(),
            spacer(),
            button("Simple Snackbar").action(|State(m): State<SnackbarManager>| {
                m.show(Snackbar::new("Hello from Snackbar!"));
            }),
            button("With Icon").action(|State(m): State<SnackbarManager>| {
                m.show(Snackbar::new("File saved successfully").icon(system_icon::checkmark()));
            }),
            button("With Action Button").action(|State(m): State<SnackbarManager>| {
                m.show(
                    Snackbar::new("Item moved to trash")
                        .icon(system_icon::trash())
                        .duration(Duration::from_secs(5))
                        .action("Undo", || {
                            waterui::log::info!("Undo clicked!");
                        }),
                );
            }),
            button("Top Position").action(|State(m): State<SnackbarManager>| {
                m.show(
                    Snackbar::new("Network connected")
                        .icon(system_icon::checkmark())
                        .position(SnackbarPosition::TopCenter),
                );
            }),
            button("Queue Multiple").action(|State(m): State<SnackbarManager>| {
                m.show(Snackbar::new("First message"));
                m.show(Snackbar::new("Second message"));
                m.show(Snackbar::new("Third message"));
            }),
            spacer(),
        ))
        .padding(),
    )
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}
