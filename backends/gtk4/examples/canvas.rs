//! Example showing how to use the canvas component in a GTK4 application.

use gtk4::prelude::*;
use vello::peniko::Color;
use waterui::Environment;
use waterui_canvas::{DynamicCanvas, canvas_with_context, GraphicsContext};
use waterui_gtk4::renderer::render;

fn main() {
    // Create the application
    let app = gtk4::Application::new(
        Some("com.example.waterui-canvas"),
        gtk4::gio::ApplicationFlags::empty(),
    );

    app.connect_activate(|app| {
        // Create the main window after the app is activated
        let window = gtk4::ApplicationWindow::new(app);

        window.set_title(Some("WaterUI Canvas Example"));
        window.set_default_size(500, 400);

        // Create the canvas view with closure-based drawing API
        let canvas_view: DynamicCanvas = canvas_with_context(400.0, 300.0, |context| {
            // Clear background with white
            context.clear(Color::WHITE);
            
            // Draw a red rectangle
            context.fill_rect(
                GraphicsContext::rect(50.0, 50.0, 100.0, 80.0), 
                Color::rgb(1.0, 0.2, 0.2)
            );
            
            // Draw a green circle
            context.fill_circle(
                GraphicsContext::point(250.0, 100.0), 
                40.0, 
                Color::rgb(0.2, 1.0, 0.2)
            );
            
            // Draw a blue line
            context.stroke_line(
                GraphicsContext::point(100.0, 200.0),
                GraphicsContext::point(300.0, 250.0),
                Color::rgb(0.2, 0.2, 1.0),
                5.0
            );
            
            // Add some transformations to demonstrate the power of the context API
            context.with_save(|ctx| {
                ctx.translate(150.0, 150.0);
                ctx.rotate(0.5); // Rotate by 0.5 radians
                
                // Draw a rotated yellow rectangle
                ctx.fill_rect(
                    GraphicsContext::rect(-25.0, -25.0, 50.0, 50.0),
                    Color::rgb(1.0, 1.0, 0.2)
                );
            });
        });

        // Render the canvas view using the GTK4 backend
        let env = Environment::new();
        let widget = render(canvas_view, &env);

        // Add the widget to the window
        window.set_child(Some(&widget));

        // Show the window
        window.present();

        println!("Canvas example running - showing rectangle, circle, and line");
    });

    // Run the application
    app.run();
}
