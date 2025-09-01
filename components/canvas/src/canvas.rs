//! Canvas component implementation with drawing utilities.

use crate::{CanvasView, Drawable, GraphicsContext};
use alloc::{vec::Vec, sync::Arc};
use vello::{
    Scene,
    kurbo::{BezPath, Circle, Line, Point, Rect, Stroke},
    peniko::{Brush, Color, Fill},
};

/// A collection of drawable elements that can be rendered on a canvas.
#[derive(Debug, Clone)]
pub struct CanvasContent {
    /// List of drawable elements
    elements: Vec<DrawElement>,
}

/// Enum representing different types of drawable elements.
#[derive(Debug, Clone)]
enum DrawElement {
    Rectangle(Rectangle),
    Circle(CircleShape),
    Line(LineShape),
    Path(PathShape),
}

impl CanvasContent {
    /// Creates a new empty canvas content.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            elements: Vec::new(),
        }
    }

    /// Adds a drawable element to the canvas content.
    fn add_element(mut self, element: DrawElement) -> Self {
        self.elements.push(element);
        self
    }

    /// Draws a rectangle at the specified position with given dimensions.
    #[must_use]
    pub fn rect(self, x: f32, y: f32, width: f32, height: f32, color: Color) -> Self {
        self.add_element(DrawElement::Rectangle(Rectangle {
            rect: Rect::new(
                f64::from(x),
                f64::from(y),
                f64::from(x + width),
                f64::from(y + height),
            ),
            brush: Brush::Solid(color),
        }))
    }

    /// Draws a circle at the specified center with given radius.
    #[must_use]
    pub fn circle(self, center_x: f32, center_y: f32, radius: f32, color: Color) -> Self {
        self.add_element(DrawElement::Circle(CircleShape {
            circle: Circle::new(
                Point::new(f64::from(center_x), f64::from(center_y)),
                f64::from(radius),
            ),
            brush: Brush::Solid(color),
        }))
    }

    /// Draws a line from start point to end point.
    #[must_use]
    pub fn line(
        self,
        start_x: f32,
        start_y: f32,
        end_x: f32,
        end_y: f32,
        color: Color,
        width: f32,
    ) -> Self {
        self.add_element(DrawElement::Line(LineShape {
            line: Line::new(
                Point::new(f64::from(start_x), f64::from(start_y)),
                Point::new(f64::from(end_x), f64::from(end_y)),
            ),
            stroke: Stroke::new(f64::from(width)),
            brush: Brush::Solid(color),
        }))
    }

    /// Draws a bezier path.
    #[must_use]
    pub fn path(
        self,
        path: BezPath,
        brush: Brush,
        stroke_style: Option<Stroke>,
        stroke_brush: Option<Brush>,
    ) -> Self {
        self.add_element(DrawElement::Path(PathShape {
            path,
            brush,
            stroke_style,
            stroke_brush,
        }))
    }
}

impl Default for CanvasContent {
    fn default() -> Self {
        Self::new()
    }
}

impl Drawable for CanvasContent {
    fn draw(&self, scene: &mut Scene) {
        for element in &self.elements {
            match element {
                DrawElement::Rectangle(rect) => rect.draw_element(scene),
                DrawElement::Circle(circle) => circle.draw_element(scene),
                DrawElement::Line(line) => line.draw_element(scene),
                DrawElement::Path(path) => path.draw_element(scene),
            }
        }
    }
}

/// A closure-based canvas content that allows dynamic drawing.
/// Provides a function-based API for flexible drawing operations.
#[derive(Clone)]
pub struct DynamicCanvasContent {
    draw_fn: Arc<dyn Fn(&mut GraphicsContext) + Send + Sync>,
}

impl DynamicCanvasContent {
    /// Creates a new dynamic canvas content with the given drawing function.
    pub fn new<F>(draw_fn: F) -> Self 
    where 
        F: Fn(&mut GraphicsContext) + Send + Sync + 'static,
    {
        Self {
            draw_fn: Arc::new(draw_fn),
        }
    }
}

impl core::fmt::Debug for DynamicCanvasContent {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DynamicCanvasContent")
            .field("draw_fn", &"<closure>")
            .finish()
    }
}

impl Drawable for DynamicCanvasContent {
    fn draw(&self, scene: &mut Scene) {
        let mut context = GraphicsContext::new(scene);
        (self.draw_fn)(&mut context);
    }
}

/// Trait for individual drawable elements.
pub trait DrawableElement: Send + Sync + core::fmt::Debug {
    /// Draw this element onto the scene.
    fn draw_element(&self, scene: &mut Scene);
}

/// A drawable rectangle element.
#[derive(Debug, Clone)]
pub struct Rectangle {
    rect: Rect,
    brush: Brush,
}

impl DrawableElement for Rectangle {
    fn draw_element(&self, scene: &mut Scene) {
        scene.fill(
            Fill::NonZero,
            vello::kurbo::Affine::IDENTITY,
            &self.brush,
            None,
            &self.rect,
        );
    }
}

/// A drawable circle element.
#[derive(Debug, Clone)]
pub struct CircleShape {
    circle: Circle,
    brush: Brush,
}

impl DrawableElement for CircleShape {
    fn draw_element(&self, scene: &mut Scene) {
        scene.fill(
            Fill::NonZero,
            vello::kurbo::Affine::IDENTITY,
            &self.brush,
            None,
            &self.circle,
        );
    }
}

/// A drawable line element.
#[derive(Debug, Clone)]
pub struct LineShape {
    line: Line,
    stroke: Stroke,
    brush: Brush,
}

impl DrawableElement for LineShape {
    fn draw_element(&self, scene: &mut Scene) {
        scene.stroke(
            &self.stroke,
            vello::kurbo::Affine::IDENTITY,
            &self.brush,
            None,
            &self.line,
        );
    }
}

/// A drawable path element.
#[derive(Debug, Clone)]
pub struct PathShape {
    path: BezPath,
    brush: Brush,
    stroke_style: Option<Stroke>,
    stroke_brush: Option<Brush>,
}

impl DrawableElement for PathShape {
    fn draw_element(&self, scene: &mut Scene) {
        if let (Some(stroke_style), Some(stroke_brush)) = (&self.stroke_style, &self.stroke_brush) {
            scene.stroke(
                stroke_style,
                vello::kurbo::Affine::IDENTITY,
                stroke_brush,
                None,
                &self.path,
            );
        }
        scene.fill(
            Fill::NonZero,
            vello::kurbo::Affine::IDENTITY,
            &self.brush,
            None,
            &self.path,
        );
    }
}

/// Creates a new canvas with the specified dimensions and content.
pub const fn canvas<T: Drawable + Clone + 'static>(
    content: T,
    width: f32,
    height: f32,
) -> CanvasView<T> {
    CanvasView::new(content, width, height)
}

/// Creates a new dynamic canvas with a drawing closure for flexible rendering.
/// 
/// # Example
/// ```rust
/// use waterui_canvas::{canvas_with_context, GraphicsContext};
/// use vello::peniko::Color;
/// 
/// let canvas_view = canvas_with_context(400.0, 300.0, |context| {
///     // Clear background
///     context.clear(Color::WHITE);
///     
///     // Draw a red rectangle
///     context.fill_rect(
///         context.rect(50.0, 50.0, 100.0, 80.0), 
///         Color::rgb(1.0, 0.2, 0.2)
///     );
///     
///     // Draw a green circle
///     context.fill_circle(
///         context.point(250.0, 100.0), 
///         40.0, 
///         Color::rgb(0.2, 1.0, 0.2)
///     );
///     
///     // Draw a blue line
///     context.stroke_line(
///         context.point(100.0, 200.0),
///         context.point(300.0, 250.0),
///         Color::rgb(0.2, 0.2, 1.0),
///         5.0
///     );
/// });
/// ```
pub fn canvas_with_context<F>(
    width: f32, 
    height: f32, 
    draw_fn: F
) -> CanvasView<DynamicCanvasContent>
where
    F: Fn(&mut GraphicsContext) + Send + Sync + 'static,
{
    let content = DynamicCanvasContent::new(draw_fn);
    CanvasView::new(content, width, height)
}
