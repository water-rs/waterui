//! Multi-scene example — exercises per-scene window state on iPadOS.
//!
//! Run on a multi-scene platform with `--waterui-e2e-second-scene`: every
//! scene the system connects builds this content again with its own snackbar
//! manager, so the toast the first scene shows never appears in the second
//! window.

use core::time::Duration;
use waterui::app::App;
use waterui::prelude::*;
use waterui::preview;
use waterui::snackbar::{Snackbar, SnackbarManager};

#[preview]
pub fn demo() -> impl View {
    vstack((
        text("Multi-Scene").title().bold(),
        text("Every scene builds this content fresh."),
        button("Show snackbar").action(|manager: SnackbarManager| {
            manager.show(Snackbar::new("Only this scene's snackbar").duration(Duration::ZERO));
        }),
        spacer(),
    ))
    .padding()
}

pub fn app(env: Environment) -> App {
    App::new(demo, env)
}
