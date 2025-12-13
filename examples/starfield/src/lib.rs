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
        text("Flame Animation").size(24),
        text("GPU-rendered procedural fire").size(14),
        // Just one line to load and render a shader!
        shader!("starfield.wgsl").size(400.0, 500.0),
        text("Rendered at 120fps").size(12),
    ))
    .padding()
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
