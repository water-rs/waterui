//! Graphics context for canvas drawing operations.

use vello::{
    Scene,
    kurbo::{Affine, BezPath, Circle, Line, Point, Rect, Stroke},
    peniko::{Brush, Color, Fill},
};

/// A graphics context for drawing operations on a canvas.
/// Provides low-level drawing commands and transform operations.
pub struct GraphicsContext<'a> {
    scene: &'a mut Scene,
    transform: Affine,
}

impl core::fmt::Debug for GraphicsContext<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GraphicsContext")
            .field("transform", &self.transform)
            .field("scene", &"<Scene>")
            .finish()
    }
}

impl<'a> GraphicsContext<'a> {
    /// Creates a new graphics context with the given scene.
    pub const fn new(scene: &'a mut Scene) -> Self {
        Self {
            scene,
            transform: Affine::IDENTITY,
        }
    }

    /// Fills a rectangle with the specified color.
    pub fn fill_rect(&mut self, rect: impl Into<Rect>, color: Color) {
        let rect = rect.into();
        let brush = Brush::Solid(color);
        self.scene
            .fill(Fill::NonZero, self.transform, &brush, None, &rect);
    }

    /// Fills a circle with the specified color.
    pub fn fill_circle(&mut self, center: Point, radius: f64, color: Color) {
        let circle = Circle::new(center, radius);
        let brush = Brush::Solid(color);
        self.scene
            .fill(Fill::NonZero, self.transform, &brush, None, &circle);
    }

    /// Strokes a line with the specified color and width.
    pub fn stroke_line(&mut self, start: Point, end: Point, color: Color, width: f64) {
        let line = Line::new(start, end);
        let stroke = Stroke::new(width);
        let brush = Brush::Solid(color);
        self.scene
            .stroke(&stroke, self.transform, &brush, None, &line);
    }

    /// Strokes a rectangle with the specified color and width.
    pub fn stroke_rect(&mut self, rect: impl Into<Rect>, color: Color, width: f64) {
        let rect = rect.into();
        let stroke = Stroke::new(width);
        let brush = Brush::Solid(color);
        self.scene
            .stroke(&stroke, self.transform, &brush, None, &rect);
    }

    /// Strokes a circle with the specified color and width.
    pub fn stroke_circle(&mut self, center: Point, radius: f64, color: Color, width: f64) {
        let circle = Circle::new(center, radius);
        let stroke = Stroke::new(width);
        let brush = Brush::Solid(color);
        self.scene
            .stroke(&stroke, self.transform, &brush, None, &circle);
    }

    /// Fills a path with the specified brush.
    pub fn fill_path(&mut self, path: &BezPath, brush: &Brush) {
        self.scene
            .fill(Fill::NonZero, self.transform, brush, None, path);
    }

    /// Strokes a path with the specified brush and stroke style.
    pub fn stroke_path(&mut self, path: &BezPath, brush: &Brush, stroke: &Stroke) {
        self.scene.stroke(stroke, self.transform, brush, None, path);
    }

    /// Applies a translation to the current transform.
    pub fn translate(&mut self, dx: f64, dy: f64) {
        self.transform *= Affine::translate((dx, dy));
    }

    /// Applies a rotation to the current transform.
    pub fn rotate(&mut self, angle: f64) {
        self.transform *= Affine::rotate(angle);
    }

    /// Applies a scale to the current transform.
    pub fn scale(&mut self, sx: f64, sy: f64) {
        self.transform *= Affine::scale_non_uniform(sx, sy);
    }

    /// Saves the current transform state and executes the closure.
    pub fn with_save<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let saved_transform = self.transform;
        let result = f(self);
        self.transform = saved_transform;
        result
    }

    /// Sets the current transform.
    pub const fn set_transform(&mut self, transform: Affine) {
        self.transform = transform;
    }

    /// Gets the current transform.
    #[must_use]
    pub const fn transform(&self) -> Affine {
        self.transform
    }

    /// Clears the canvas with the specified color.
    pub fn clear(&mut self, color: Color) {
        // Create a large rectangle to cover the entire canvas
        let large_rect = Rect::new(-1e6, -1e6, 1e6, 1e6);
        let brush = Brush::Solid(color);
        self.scene
            .fill(Fill::NonZero, Affine::IDENTITY, &brush, None, &large_rect);
    }
}

/// Convenience functions for creating common shapes and points.
impl GraphicsContext<'_> {
    /// Creates a rectangle from x, y, width, height.
    #[must_use]
    pub fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect::new(x, y, x + width, y + height)
    }

    /// Creates a point from x, y coordinates.
    #[must_use]
    pub const fn point(x: f64, y: f64) -> Point {
        Point::new(x, y)
    }
}
