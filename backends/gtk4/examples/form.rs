//! Form example demonstrating various input widgets.

use waterui::{
    Binding,
    component::{
        form::{TextField, field, slider::slider, stepper, toggle},
        layout::stack::vstack,
        text,
    },
    core::binding,
};
use waterui_gtk4::{Gtk4App, init};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize GTK4
    init()?;

    // Create the application
    let app = Gtk4App::new("com.example.waterui-gtk4-form");

    // Run the application
    let exit_code = app.run(|| {
        // Create reactive state for form fields
        let name = binding("John Doe");
        let email = binding("john@example.com");
        let age = binding(25);
        let newsletter = binding(true);
        let volume = binding(50.0);

        // Build the form UI - using AnyView for mixed types
        vstack((
            "User Registration Form",
            "=== Form Components Demo ===",
            // Simple form fields that should render via GTK4 backend
            field("Name:", &name),
            field("Email:", &email),
            field("", &email),
            toggle("Subscribe to Newsletter", &newsletter),
            stepper(&age),
            slider(0.0..=100.0, &volume),
            "=== Current Values ===",
            text!("Name: {}", name),
            text!("Email: {}", email),
            text!("Age: {}", age),
            text!("Newsletter: {}", newsletter),
            text!("Volume: {:.0}%", volume),
        ))
    });

    if exit_code != 0 {
        eprintln!("Application exited with code: {}", exit_code);
    }

    Ok(())
}
