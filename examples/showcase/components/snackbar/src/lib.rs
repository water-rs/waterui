//! Snackbar test playground

use core::time::Duration;
use waterui::app::App;
use waterui::prelude::*;
use waterui::snackbar::{Snackbar, SnackbarManager, SnackbarPosition};
use waterui_icon::system_icon;

fn main(manager: SnackbarManager) -> impl View {
    scroll(
        vstack((
            text("Snackbar Demo").title().bold(),
            spacer(),
            button("Simple Snackbar")
                .action(|State(m): State<SnackbarManager>| {
                    m.show(Snackbar::new("Hello from Snackbar!"));
                })
                .state(&manager),
            button("With Icon")
                .action(|State(m): State<SnackbarManager>| {
                    m.show(Snackbar::new("File saved successfully").icon(system_icon::checkmark()));
                })
                .state(&manager),
            button("With Action Button")
                .action(|State(m): State<SnackbarManager>| {
                    m.show(
                        Snackbar::new("Item moved to trash")
                            .icon(system_icon::trash())
                            .duration(Duration::from_secs(5))
                            .action("Undo", || {
                                waterui::log::info!("Undo clicked!");
                            }),
                    );
                })
                .state(&manager),
            button("Top Position")
                .action(|State(m): State<SnackbarManager>| {
                    m.show(
                        Snackbar::new("Network connected")
                            .icon(system_icon::checkmark())
                            .position(SnackbarPosition::TopCenter),
                    );
                })
                .state(&manager),
            button("Queue Multiple")
                .action(|State(m): State<SnackbarManager>| {
                    m.show(Snackbar::new("First message"));
                    m.show(Snackbar::new("Second message"));
                    m.show(Snackbar::new("Third message"));
                })
                .state(&manager),
            spacer(),
        ))
        .padding(),
    )
}

pub fn app(env: Environment) -> App {
    let manager = env
        .get::<SnackbarManager>()
        .expect("SnackbarManager not found")
        .clone();
    App::new(move || main(manager.clone()), env)
}
