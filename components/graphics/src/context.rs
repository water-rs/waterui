use alloc::vec::Vec;
use core::fmt::{self, Debug};

use waterui_color::Color;
use waterui_core::Environment;

use crate::shape::{DrawStyle, Path};

/// A recorded drawing command issued by the [`GraphicsContext`].
#[derive(Debug, Clone)]
pub(crate) struct DrawCommand {
    pub(crate) path: Path,
    pub(crate) style: DrawStyle,
}

/// A context for issuing 2D drawing commands.
///
/// User code interacts with the `GraphicsContext` to describe shapes in a backend
/// agnostic fashion. The recorded commands are later consumed by the renderer
/// implementation (for example, the CPU renderer based on `tiny-skia`).
pub struct GraphicsContext<'a> {
    env: &'a Environment,
    commands: Vec<DrawCommand>,
}

impl Debug for GraphicsContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GraphicsContext")
            .field("command_count", &self.commands.len())
            .finish()
    }
}

impl<'a> GraphicsContext<'a> {
    /// Creates a new graphics context bound to the provided environment.
    pub(crate) fn new(env: &'a Environment) -> Self {
        Self {
            env,
            commands: Vec::new(),
        }
    }

    /// Returns the environment associated with this context.
    #[must_use]
    pub fn environment(&self) -> &Environment {
        self.env
    }

    /// Draws a path with a given style (fill or stroke).
    pub fn draw(&mut self, path: &Path, style: &DrawStyle) {
        self.commands.push(DrawCommand {
            path: path.clone(),
            style: style.clone(),
        });
    }

    /// Fills a path with a solid color.
    pub fn fill(&mut self, path: &Path, color: &Color) {
        self.draw(path, &DrawStyle::Fill(color.clone()));
    }

    /// Strokes a path with a solid color and width.
    pub fn stroke(&mut self, path: &Path, color: &Color, width: f32) {
        self.draw(path, &DrawStyle::Stroke(color.clone(), width));
    }

    /// Consumes the context, returning the recorded drawing commands.
    pub(crate) fn into_commands(self) -> Vec<DrawCommand> {
        self.commands
    }
}

pub(crate) use DrawCommand as RecordedCommand;
