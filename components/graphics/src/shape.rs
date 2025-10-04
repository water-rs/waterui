use kurbo::{BezPath, Point};
use waterui_color::Color;

/// A backend-agnostic representation of a 2D path command.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathCommand {
    MoveTo([f32; 2]),
    LineTo([f32; 2]),
    QuadTo([f32; 2], [f32; 2]),
    CurveTo([f32; 2], [f32; 2], [f32; 2]),
    Close,
}

/// A list of path commands that describe a shape.
/// This is the public, backend-agnostic representation of a path.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Path(pub Vec<PathCommand>);

impl Path {
    /// Creates a new, empty path builder.
    pub fn builder() -> PathBuilder {
        PathBuilder::new()
    }

    /// Internal function to convert our Path to a Vello-compatible BezPath.
    /// This is kept private to avoid exposing backend details.
    pub(crate) fn to_kurbo(&self) -> kurbo::BezPath {
        let mut bez_path = BezPath::new();
        for command in &self.0 {
            match *command {
                PathCommand::MoveTo(p) => bez_path.move_to(Point::new(p[0] as f64, p[1] as f64)),
                PathCommand::LineTo(p) => bez_path.line_to(Point::new(p[0] as f64, p[1] as f64)),
                PathCommand::QuadTo(p1, p2) => bez_path.quad_to(
                    Point::new(p1[0] as f64, p1[1] as f64),
                    Point::new(p2[0] as f64, p2[1] as f64),
                ),
                PathCommand::CurveTo(p1, p2, p3) => bez_path.curve_to(
                    Point::new(p1[0] as f64, p1[1] as f64),
                    Point::new(p2[0] as f64, p2[1] as f64),
                    Point::new(p3[0] as f64, p3[1] as f64),
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
    pub fn new() -> Self {
        Self::default()
    }

    pub fn move_to(mut self, point: [f32; 2]) -> Self {
        self.0.push(PathCommand::MoveTo(point));
        self
    }

    pub fn line_to(mut self, point: [f32; 2]) -> Self {
        self.0.push(PathCommand::LineTo(point));
        self
    }

    pub fn quad_to(mut self, p1: [f32; 2], p2: [f32; 2]) -> Self {
        self.0.push(PathCommand::QuadTo(p1, p2));
        self
    }

    pub fn curve_to(mut self, p1: [f32; 2], p2: [f32; 2], p3: [f32; 2]) -> Self {
        self.0.push(PathCommand::CurveTo(p1, p2, p3));
        self
    }

    pub fn close(mut self) -> Self {
        self.0.push(PathCommand::Close);
        self
    }

    pub fn build(self) -> Path {
        Path(self.0)
    }
}

/// Defines the style for filling or stroking a shape.
#[derive(Debug, Clone)]
pub enum DrawStyle {
    Fill(Color),
    Stroke(Color, f64), // Color and width
}
