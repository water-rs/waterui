//! Basic usage examples for waterui-canvas crate.

use vello::peniko::Color;
use waterui_canvas::{CanvasContent, GraphicsContext, canvas, canvas_with_context};

fn main() {
    // Example 1: Static canvas using builder pattern
    println!("=== Static Canvas Example ===");
    let content = CanvasContent::new()
        .rect(10.0, 10.0, 100.0, 50.0, Color::rgb(1.0, 0.0, 0.0))
        .circle(150.0, 50.0, 30.0, Color::rgb(0.0, 1.0, 0.0))
        .line(200.0, 20.0, 300.0, 80.0, Color::rgb(0.0, 0.0, 1.0), 3.0);

    let _static_canvas = canvas(content, 400.0, 200.0);
    println!("Static canvas view created with rectangle, circle, and line!");

    // Example 2: Dynamic canvas using closure-based API
    println!("\n=== Dynamic Canvas Example (Closure-based) ===");
    let _dynamic_canvas = canvas_with_context(400.0, 300.0, |context| {
        // Clear background
        context.clear(Color::WHITE);

        // Draw shapes with full control over the graphics context
        context.fill_rect(
            GraphicsContext::rect(50.0, 50.0, 100.0, 80.0),
            Color::rgb(1.0, 0.2, 0.2),
        );

        context.fill_circle(
            GraphicsContext::point(250.0, 100.0),
            40.0,
            Color::rgb(0.2, 1.0, 0.2),
        );

        context.stroke_line(
            GraphicsContext::point(100.0, 200.0),
            GraphicsContext::point(300.0, 250.0),
            Color::rgb(0.2, 0.2, 1.0),
            5.0,
        );

        // Demonstrate transformations
        context.with_save(|ctx| {
            ctx.translate(200.0, 200.0);
            ctx.rotate(0.785); // 45 degrees
            ctx.scale(1.5, 0.8);

            ctx.fill_rect(
                GraphicsContext::rect(-25.0, -25.0, 50.0, 50.0),
                Color::rgb(1.0, 1.0, 0.2),
            );
        });
    });

    println!("Dynamic canvas view created with context-based drawing!");
    println!("The dynamic API allows for more flexible and interactive drawing operations.");
}
