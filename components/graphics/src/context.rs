use crate::shape::{DrawStyle, Path};
use kurbo::Affine;
use vello::{
    Scene,
    peniko::{self, kurbo::Stroke},
};
use waterui_color::{Color, ResolvedColor};
use waterui_core::{Environment, Signal};

/// A context for issuing 2D drawing commands.
///
/// This acts as a public-facing API that wraps an internal Vello scene,
/// allowing users to draw without needing to know about the backend.
pub struct GraphicsContext<'a> {
    pub(crate) scene: &'a mut Scene,
    pub(crate) env: &'a Environment,
}

impl<'a> GraphicsContext<'a> {
    /// Draws a path with a given style (fill or stroke).
    pub fn draw(&mut self, path: &Path, style: &DrawStyle) {
        let kurbo_path = path.to_kurbo();

        match style {
            DrawStyle::Fill(color) => {
                self.scene.fill(
                    peniko::Fill::NonZero,
                    Affine::IDENTITY,
                    &to_peniko_brush(&color.resolve(self.env).get()),
                    None,
                    &kurbo_path,
                );
            }
            DrawStyle::Stroke(color, width) => {
                self.scene.stroke(
                    &Stroke::new(*width),
                    Affine::IDENTITY,
                    &to_peniko_brush(&color.resolve(self.env).get()),
                    None,
                    &kurbo_path,
                );
            }
        }
    }

    // Convenience methods

    /// Fills a path with a solid color.
    pub fn fill(&mut self, path: &Path, color: &Color) {
        self.draw(path, &DrawStyle::Fill(color.clone()));
    }

    /// Strokes a path with a solid color and width.
    pub fn stroke(&mut self, path: &Path, color: &Color, width: f64) {
        self.draw(path, &DrawStyle::Stroke(color.clone(), width));
    }
}

/// Internal utility to convert WaterUI resolved color to Vello brush.
fn to_peniko_brush(color: &ResolvedColor) -> peniko::Brush {
    peniko::Brush::Solid(peniko::Color::rgba(
        color.red as f64,
        color.green as f64,
        color.blue as f64,
        color.opacity as f64,
    ))
}
