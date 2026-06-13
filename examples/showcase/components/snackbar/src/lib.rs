//! Snackbar test playground

use core::time::Duration;
use waterui::app::App;
use waterui::prelude::*;
use waterui::preview;
use waterui::snackbar::{Snackbar, SnackbarManager, SnackbarPosition};
use waterui_icons_material_icon as mdi;

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
                m.show(Snackbar::new("File saved successfully").icon(mdi::check_circle()));
            }),
            button("With Action Button").action(|State(m): State<SnackbarManager>| {
                m.show(
                    Snackbar::new("Item moved to trash")
                        .icon(mdi::delete())
                        .duration(Duration::from_secs(5))
                        .action("Undo", || {
                            waterui::log::info!("Undo clicked!");
                        }),
                );
            }),
            button("Top Position").action(|State(m): State<SnackbarManager>| {
                m.show(
                    Snackbar::new("Network connected")
                        .icon(mdi::check_circle())
                        .position(SnackbarPosition::TopCenter),
                );
            }),
            button("Queue Multiple").action(|State(m): State<SnackbarManager>| {
                m.show(Snackbar::new("First message"));
                m.show(Snackbar::new("Second message"));
                m.show(Snackbar::new("Third message"));
            }),
            button("Top + Bottom").action(|State(m): State<SnackbarManager>| {
                // Different placements are independent — these coexist.
                m.show(
                    Snackbar::new("Top banner")
                        .icon(mdi::check_circle())
                        .position(SnackbarPosition::TopCenter),
                );
                m.show(Snackbar::new("Bottom banner"));
            }),
            button("Closeable").action(|State(m): State<SnackbarManager>| {
                m.show(
                    Snackbar::new("Stays until you close it")
                        .duration(Duration::ZERO)
                        .closeable(),
                );
            }),
            spacer(),
        ))
        .padding(),
    )
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}
