//! Snackbar test example

use core::time::Duration;
use mdi::check_circle;
use mdi::delete;
use waterui::log::info;
use waterui::prelude::*;
use waterui::preview;
use waterui::snackbar::{Snackbar, SnackbarManager, SnackbarPosition};
use waterui_icons_material_icon as mdi;

type IconSnackbarManager = SnackbarManager<mdi::Icon>;

/// The snackbar example page, with buttons that show each snackbar variant.
#[preview]
pub fn demo() -> impl View {
    scroll(
        vstack((
            text("Snackbar Demo").title().bold(true),
            spacer(),
            button("Simple Snackbar").action(|m: IconSnackbarManager| {
                m.show(Snackbar::new("Hello from Snackbar!"));
            }),
            button("With Icon").action(|m: IconSnackbarManager| {
                m.show(Snackbar::new("File saved successfully").icon(check_circle));
            }),
            button("With Action Button").action(|m: IconSnackbarManager| {
                m.show(
                    Snackbar::new("Item moved to trash")
                        .icon(delete)
                        .duration(Duration::from_secs(5))
                        .action("Undo", || {
                            info!("Undo clicked!");
                        }),
                );
            }),
            button("Top Position").action(|m: IconSnackbarManager| {
                m.show(
                    Snackbar::new("Network connected")
                        .icon(check_circle)
                        .position(SnackbarPosition::TopCenter),
                );
            }),
            button("Queue Multiple").action(|m: IconSnackbarManager| {
                m.show(Snackbar::new("First message"));
                m.show(Snackbar::new("Second message"));
                m.show(Snackbar::new("Third message"));
            }),
            button("Top + Bottom").action(|m: IconSnackbarManager| {
                // Different placements are independent — these coexist.
                m.show(
                    Snackbar::new("Top banner")
                        .icon(check_circle)
                        .position(SnackbarPosition::TopCenter),
                );
                m.show(Snackbar::new("Bottom banner"));
            }),
            button("Closeable").action(|m: IconSnackbarManager| {
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
    .snackbar_host_with_icons::<mdi::Icon>()
}

/// Builds the snackbar example app.
pub fn app(env: Environment) -> App<impl Windows> {
    App::new(env).window(Window::new("", demo))
}
