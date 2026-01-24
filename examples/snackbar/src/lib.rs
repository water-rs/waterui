//! Snackbar test playground

use core::time::Duration;
use waterui::app::App;
use waterui::prelude::*;
use waterui::snackbar::{Snackbar, SnackbarManager, SnackbarPosition};
use waterui_icon::SystemIcon;

fn main(env: &Environment) -> impl View {
    let manager = env
        .get::<SnackbarManager>()
        .expect("SnackbarManager not found")
        .clone();

    scroll(
        vstack((
            text("Snackbar Demo").title().bold(),
            spacer(),
            button("Simple Snackbar").with_state(&manager).action(|m| {
                m.show(Snackbar::new("Hello from Snackbar!"));
            }),
            button("With Icon").with_state(&manager).action(|m| {
                m.show(Snackbar::new("File saved successfully").icon(SystemIcon::CHECKMARK));
            }),
            button("With Action Button")
                .with_state(&manager)
                .action(|m| {
                    m.show(
                        Snackbar::new("Item moved to trash")
                            .icon(SystemIcon::TRASH)
                            .duration(Duration::from_secs(5))
                            .action("Undo")
                            .handler(|| {
                                waterui::log::info!("Undo clicked!");
                            }),
                    );
                }),
            button("Top Position").with_state(&manager).action(|m| {
                m.show(
                    Snackbar::new("Network connected")
                        .icon(SystemIcon::CHECKMARK)
                        .position(SnackbarPosition::TopCenter),
                );
            }),
            button("Queue Multiple").with_state(&manager).action(|m| {
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
    App::new(main(&env), env)
}

waterui_ffi::export!();
