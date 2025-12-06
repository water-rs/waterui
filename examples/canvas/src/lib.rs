//! Canvas demo - 2D vector graphics with Vello.
//!
//! This example demonstrates the Canvas API for drawing shapes,
//! paths, and animations using GPU-accelerated 2D rendering.

use waterui::graphics::Canvas;
use waterui::graphics::kurbo::{Circle, Point, Rect, RoundedRect};
use waterui::graphics::peniko::Color;
use waterui::prelude::*;

pub fn init() -> Environment {
    Environment::new()
}

pub fn main() -> impl View {
    vstack((
        text("Canvas Demo").size(24),
        text("GPU-accelerated 2D graphics with Vello").size(14),
        // Canvas with drawing callback
        Canvas::new(|ctx| {
            let center = ctx.center();
            let size = ctx.size();

            // Background
            ctx.fill(
                Rect::from_origin_size(Point::ZERO, size),
                Color::new([0.1, 0.1, 0.15, 1.0]),
            );

            // Rounded rectangle
            ctx.fill(
                RoundedRect::new(20.0, 20.0, 200.0, 120.0, 12.0),
                Color::new([0.3, 0.5, 0.8, 1.0]),
            );

            // Circle in center
            ctx.fill(Circle::new(center, 60.0), Color::new([0.9, 0.3, 0.4, 1.0]));

            // Stroked circle
            ctx.stroke(
                Circle::new(center, 80.0),
                Color::new([1.0, 1.0, 1.0, 0.8]),
                3.0,
            );

            // Small decorative circles
            for i in 0..8 {
                let angle = (i as f64) * std::f64::consts::PI / 4.0;
                let x = center.x + 120.0 * angle.cos();
                let y = center.y + 120.0 * angle.sin();
                ctx.fill(Circle::new((x, y), 10.0), Color::new([0.8, 0.8, 0.2, 1.0]));
            }
        }),
        text("Rendered at 120fps").size(12),
    ))
    .padding()
}

waterui_ffi::export!();
