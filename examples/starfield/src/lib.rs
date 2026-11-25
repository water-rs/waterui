//! Starfield animation example using `ShaderSurface`.
//!
//! This example demonstrates the simplest way to create GPU-rendered content
//! using the `shader!` macro.
//!
//! The starfield effect uses a GPU shader to render layered stars and nebulae.

use waterui::app::{App, Windows};
use waterui::graphics::shader;
use waterui::prelude::*;
use waterui::preview;

/// The starfield demo: an animated shader surface under a title and a caption.
#[preview]
pub fn demo() -> impl View {
    vstack((
        text("Starfield Animation")
            .font_size(24)
            .foreground(Color::srgb(245, 247, 250)),
        text("GPU-rendered procedural starfield")
            .font_size(14)
            .foreground(Color::srgb(210, 216, 224)),
        shader!("starfield.wgsl")
            .continuous(true)
            .size(400.0, 500.0),
        text("Rendered at 120fps")
            .font_size(12)
            .foreground(Color::srgb(210, 216, 224)),
    ))
    .background(Color::srgb(31, 35, 38))
    .padding()
}

/// Builds the starfield example app around [`demo`].
pub fn app(env: Environment) -> App<impl Windows> {
    App::new(env).window(Window::new("", demo))
}
