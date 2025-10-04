use kurbo::{BezPath, Point};
use waterui_color::Color;

/// A backend-agnostic representation of a 2D path command.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathCommand {
    /// Move the current point to the specified position without drawing.
    MoveTo([f32; 2]),
    /// Draw a straight line from the current point to the specified position.
    LineTo([f32; 2]),
    /// Draw a quadratic Bézier curve with one control point.
    QuadTo([f32; 2], [f32; 2]),
    /// Draw a cubic Bézier curve with two control points.
    CurveTo([f32; 2], [f32; 2], [f32; 2]),
    /// Close the current path by drawing a line to the start point.
    Close,
}

/// A list of path commands that describe a shape.
/// This is the public, backend-agnostic representation of a path.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Path(pub Vec<PathCommand>);

impl Path {
    /// Creates a new, empty path builder.
    #[must_use] 
    pub fn builder() -> PathBuilder {
        PathBuilder::new()
    }

    /// Internal function to convert our Path to a Vello-compatible `BezPath`.
    /// This is kept private to avoid exposing backend details.
    pub(crate) fn to_kurbo(&self) -> kurbo::BezPath {
        let mut bez_path = BezPath::new();
        for command in &self.0 {
            match *command {
                PathCommand::MoveTo(p) => bez_path.move_to(Point::new(f64::from(p[0]), f64::from(p[1]))),
                PathCommand::LineTo(p) => bez_path.line_to(Point::new(f64::from(p[0]), f64::from(p[1]))),
                PathCommand::QuadTo(p1, p2) => bez_path.quad_to(
                    Point::new(f64::from(p1[0]), f64::from(p1[1])),
                    Point::new(f64::from(p2[0]), f64::from(p2[1])),
                ),
                PathCommand::CurveTo(p1, p2, p3) => bez_path.curve_to(
                    Point::new(f64::from(p1[0]), f64::from(p1[1])),
                    Point::new(f64::from(p2[0]), f64::from(p2[1])),
                    Point::new(f64::from(p3[0]), f64::from(p3[1])),
                ),
                PathCommand::Close => bez_path.close_path(),
            }
        }
        bez_path
    }
}

/// A builder for creating `Path` objects using a fluent API.
#[derive(Debug, Clone, Default)]
pub struct PathBuilder(Vec<PathCommand>);

impl PathBuilder {
    /// Creates a new, empty path builder.
    #[must_use] 
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves the current point to the specified position without drawing.
    /// 
    /// # Arguments
    /// * `point` - The [x, y] coordinates to move to
    #[must_use] 
    pub fn move_to(mut self, point: [f32; 2]) -> Self {
        self.0.push(PathCommand::MoveTo(point));
        self
    }

    /// Draws a straight line from the current point to the specified position.
    /// 
    /// # Arguments
    /// * `point` - The [x, y] coordinates to draw a line to
    #[must_use] 
    pub fn line_to(mut self, point: [f32; 2]) -> Self {
        self.0.push(PathCommand::LineTo(point));
        self
    }

    /// Draws a quadratic Bézier curve with one control point.
    /// 
    /// # Arguments
    /// * `p1` - The [x, y] coordinates of the control point
    /// * `p2` - The [x, y] coordinates of the end point
    #[must_use] 
    pub fn quad_to(mut self, p1: [f32; 2], p2: [f32; 2]) -> Self {
        self.0.push(PathCommand::QuadTo(p1, p2));
        self
    }

    /// Draws a cubic Bézier curve with two control points.
    /// 
    /// # Arguments
    /// * `p1` - The [x, y] coordinates of the first control point
    /// * `p2` - The [x, y] coordinates of the second control point
    /// * `p3` - The [x, y] coordinates of the end point
    #[must_use] 
    pub fn curve_to(mut self, p1: [f32; 2], p2: [f32; 2], p3: [f32; 2]) -> Self {
        self.0.push(PathCommand::CurveTo(p1, p2, p3));
        self
    }

    /// Closes the current path by drawing a line to the start point.
    #[must_use] 
    pub fn close(mut self) -> Self {
        self.0.push(PathCommand::Close);
        self
    }

    /// Builds and returns the completed `Path`.
    #[must_use] 
    pub fn build(self) -> Path {
        Path(self.0)
    }
}

/// Defines the style for filling or stroking a shape.
#[derive(Debug, Clone)]
pub enum DrawStyle {
    /// Fill the shape with a solid color.
    Fill(Color),
    /// Stroke the shape outline with a color and width.
    /// 
    /// The second parameter is the stroke width in pixels.
    Stroke(Color, f64),
}
