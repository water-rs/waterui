//! Canvas components for `WaterUI` using Vello for 2D graphics.
//!
//! This crate provides a canvas view that allows for 2D vector graphics rendering using the Vello library.

#![no_std]
#![allow(clippy::multiple_crate_versions)]
extern crate alloc;

use vello::Scene;
use waterui_core::{AnyView, Environment, View};

pub mod canvas;
pub mod context;

#[doc(inline)]
pub use canvas::{CanvasContent, DynamicCanvasContent, canvas, canvas_with_context};
#[doc(inline)]
pub use context::GraphicsContext;

/// Concrete canvas view type that can be registered in dispatchers.
/// This is a type alias for `CanvasView<CanvasContent>`.
pub type Canvas = CanvasView<CanvasContent>;

/// Dynamic canvas view type for closure-based drawing.
/// This is a type alias for `CanvasView<DynamicCanvasContent>`.
pub type DynamicCanvas = CanvasView<DynamicCanvasContent>;

/// A trait for drawable content that can be rendered on a canvas.
pub trait Drawable {
    /// Draw this content onto the given Vello scene.
    fn draw(&self, scene: &mut Scene);
}

/// Canvas view that supports 2D vector graphics rendering using Vello.
#[derive(Debug, Clone)]
pub struct CanvasView<T: Drawable + Clone + 'static> {
    /// The content to be drawn on the canvas
    content: T,
    /// Canvas width
    width: f32,
    /// Canvas height
    height: f32,
}

impl<T: Drawable + Clone + 'static> CanvasView<T> {
    /// Creates a new canvas view with the specified content and dimensions.
    pub const fn new(content: T, width: f32, height: f32) -> Self {
        Self {
            content,
            width,
            height,
        }
    }

    /// Sets the canvas width.
    #[must_use]
    pub const fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Sets the canvas height.
    #[must_use]
    pub const fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    /// Gets the canvas content.
    pub const fn content(&self) -> &T {
        &self.content
    }

    /// Gets the canvas dimensions as (width, height).
    pub const fn dimensions(&self) -> (f32, f32) {
        (self.width, self.height)
    }
}

impl<T: Drawable + Clone + 'static> View for CanvasView<T> {
    fn body(self, _env: &Environment) -> impl View {
        // Return self as the final view - backends will handle the actual rendering
        AnyView::new(self)
    }
}
