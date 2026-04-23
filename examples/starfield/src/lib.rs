//! Flame animation example using ShaderSurface.
//!
//! This example demonstrates the simplest way to create GPU-rendered content
//! using the `shader!` macro.
//!
//! The flame effect uses fractal Brownian motion (fBm) noise for realistic fire.

use waterui::app::App;
use waterui::graphics::shader;
use waterui::prelude::*;

fn main() -> impl View {
    vstack((
        text!("Flame Animation").title(),
        text!("GPU-rendered procedural fire").headline(),
        // Just one line to load and render a shader!
        shader!("starfield.wgsl").size(400.0, 500.0),
        text!("Rendered at 120fps"),
    ))
    .padding()
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}
