//! Example demonstrating shape rendering in WaterUI GTK4 backend.

use waterui::{
    Computed,
    component::layout::stack::vstack,
    shape::{Circle, Rectangle, RoundedRectangle},
};
use waterui_gtk4::{Gtk4App, init};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize GTK4
    init()?;

    // Create the application
    let app = Gtk4App::new("com.example.waterui-gtk4-shapes");

    // Run the application
    let exit_code = app.run(|| {
        vstack((
            // Basic rectangle (blue)
            Rectangle,
            // Rounded rectangle with 16px radius (green)
            RoundedRectangle::new(Computed::new(16.0)),
            // Circle (red)
            Circle,
        ))
    });

    std::process::exit(exit_code);
}
