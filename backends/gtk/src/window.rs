//! Window management utilities for GTK backend.

use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow};

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
