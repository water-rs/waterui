//! Painting context shared by render nodes.

use std::fmt::Debug;

use waterui_core::Environment;

use super::layout::{LayoutCtx, LayoutResult};
use crate::scene::{DrawCommand, Scene, SceneBuilder};

/// Context passed to nodes when painting into a backend-specific surface.
#[derive(Debug)]
pub struct RenderCtx<'a> {
    env: &'a Environment,
    builder: SceneBuilder,
}

impl<'a> RenderCtx<'a> {
    /// Creates a new [`RenderCtx`].
    #[must_use]
    pub fn new(env: &'a Environment) -> Self {
        Self {
            env,
            builder: SceneBuilder::new(),
        }
    }

    /// Returns the environment associated with this paint pass.
    #[must_use]
    pub const fn env(&self) -> &'a Environment {
        self.env
    }

    /// Pushes a draw command into the scene.
    pub fn push(&mut self, command: DrawCommand) {
        self.builder.push(command);
    }

    /// Finalises the recorded scene.
    #[must_use]
    pub fn finish(self) -> Scene {
        self.builder.finish()
    }
}

/// Trait implemented by every node stored in the render tree.
pub trait RenderNode: Debug {
    /// Performs layout using the provided context and returns the resulting size.
    fn layout(&mut self, ctx: LayoutCtx<'_>) -> LayoutResult;

    /// Emits draw calls into the backend-specific renderer.
    fn paint(&mut self, ctx: &mut RenderCtx<'_>);

    /// Updates reactive state. Called whenever Hydrolysis detects binding/computed changes.
    fn update_reactive(&mut self) {}
}
