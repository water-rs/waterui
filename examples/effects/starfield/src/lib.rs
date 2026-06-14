//! Starfield animation example using `ShaderSurface`.
//!
//! This example demonstrates the simplest way to create GPU-rendered content
//! using the `shader!` macro.
//!
//! The starfield effect uses a GPU shader to render layered stars and nebulae.

use waterui::app::App;
use waterui::graphics::shader;
use waterui::prelude::*;
use waterui::preview;

#[preview]
pub fn demo() -> impl View {
    vstack((
        text("Starfield Animation")
            .size(24)
            .foreground(Color::srgb(245, 247, 250)),
        text("GPU-rendered procedural starfield")
            .size(14)
            .foreground(Color::srgb(210, 216, 224)),
        shader!("starfield.wgsl").size(400.0, 500.0),
        text("Rendered at 120fps")
            .size(12)
            .foreground(Color::srgb(210, 216, 224)),
    ))
    .background(Color::srgb(31, 35, 38))
    .padding()
}

pub fn app(env: Environment) -> App {
    App::new(demo, env)
}
