//! Backend-agnostic drawing commands recorded during rendering.

use waterui_color::ResolvedColor;

use crate::tree::layout::{Point, Rect};

/// A fully recorded scene containing draw commands.
#[derive(Debug, Clone, Default)]
pub struct Scene {
    commands: Vec<DrawCommand>,
}

impl Scene {
    /// Returns the underlying commands.
    #[must_use]
    pub fn commands(&self) -> &[DrawCommand] {
        &self.commands
    }

    fn from_builder(builder: SceneBuilder) -> Self {
        Self {
            commands: builder.commands,
        }
    }
}

/// Builder used by [`RenderCtx`](crate::tree::render::RenderCtx) to capture draw commands.
#[derive(Debug, Default)]
pub struct SceneBuilder {
    commands: Vec<DrawCommand>,
}

impl SceneBuilder {
    /// Creates a new builder.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }

    /// Pushes a drawing command into the scene.
    pub fn push(&mut self, command: DrawCommand) {
        self.commands.push(command);
    }

    /// Finalises the builder, returning the immutable scene.
    #[must_use]
    pub fn finish(self) -> Scene {
        Scene::from_builder(self)
    }
}

/// Primitive drawing operations understood by Hydrolysis backends.
#[derive(Debug, Clone)]
pub enum DrawCommand {
    /// Fill a solid rectangle.
    SolidRect {
        /// Rectangle to fill (logical pixels).
        rect: Rect,
        /// Fill color resolved from the environment.
        color: ResolvedColor,
    },
    /// Draw styled text at the specified origin.
    Text {
        /// Text content to draw.
        content: String,
        /// Baseline origin for the text run.
        origin: Point,
        /// Fill color resolved from the environment.
        color: ResolvedColor,
        /// Font size in logical pixels.
        size: f32,
    },
    /// Reserved for future commands (gradients, images, strokes, etc.).
    Placeholder(&'static str),
}
