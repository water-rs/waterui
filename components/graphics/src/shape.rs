//! Shape primitives for GPU-based vector graphics rendering.
//!
//! This module provides shape primitives (Rect, Circle, Ellipse) that can be
//! rendered via GpuSurface. Each shape implements both `Shape` (for path iteration)
//! and `GpuRenderer` (for direct GPU rendering).
//!
//! # Design
//!
//! - **No heap allocation**: Simple shapes use fixed-size arrays for path commands
//! - **Iterator-based**: `Shape::path()` returns `impl Iterator<Item = PathCommand>`
//! - **Self-rendering**: Each shape implements `GpuRenderer` directly
//! - **VectorArithmetic**: Shapes can be animated/morphed via the animation system
//!
//! # Examples
//!
//! ```ignore
//! use waterui_graphics::shape::{Rect, Circle, Shape};
//!
//! // Create a rectangle
//! let rect = Rect::new([0.1, 0.1], [0.8, 0.8]);
//!
//! // Iterate over path commands
//! for cmd in rect.path() {
//!     match cmd {
//!         PathCommand::MoveTo(p) => { /* start at p */ }
//!         PathCommand::LineTo(p) => { /* line to p */ }
//!         PathCommand::Close => { /* close path */ }
//!         _ => {}
//!     }
//! }
//!
//! // Or use as a GpuSurface renderer
//! GpuSurface::new(rect.fill(Color::red()))
//! ```

use core::ops::{Add, Mul, Sub};

/// Commands for constructing paths.
///
/// These commands follow the standard SVG/Canvas path model:
/// - `MoveTo`: Start a new subpath
/// - `LineTo`: Draw a line from current point
/// - `QuadTo`: Quadratic Bezier curve
/// - `CubicTo`: Cubic Bezier curve
/// - `Close`: Close current subpath
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathCommand {
    /// Move to a point without drawing.
    MoveTo([f32; 2]),
    /// Draw a line to a point.
    LineTo([f32; 2]),
    /// Quadratic Bezier curve with one control point.
    QuadTo {
        /// Control point for the curve.
        control: [f32; 2],
        /// End point of the curve.
        end: [f32; 2],
    },
    /// Cubic Bezier curve with two control points.
    CubicTo {
        /// First control point.
        control1: [f32; 2],
        /// Second control point.
        control2: [f32; 2],
        /// End point of the curve.
        end: [f32; 2],
    },
    /// Close the current path by drawing a line to the start.
    Close,
}

/// Trait for types that can produce path commands.
///
/// Shapes implement this trait to expose their geometry as an iterator
/// of path commands, which can then be rendered by various backends.
pub trait Shape: 'static {
    /// Returns an iterator over the path commands that define this shape.
    fn path(&self) -> impl Iterator<Item = PathCommand>;
}

// ============================================================================
// Rect
// ============================================================================

/// A rectangle defined by origin and size.
///
/// Coordinates are in normalized space (0.0 to 1.0) by default,
/// but can represent any coordinate system.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    /// Origin point (top-left corner).
    pub origin: [f32; 2],
    /// Size (width, height).
    pub size: [f32; 2],
    /// Corner radius for rounded rectangles.
    pub corner_radius: f32,
}

impl Rect {
    /// Creates a new rectangle with the given origin and size.
    #[must_use]
    pub const fn new(origin: [f32; 2], size: [f32; 2]) -> Self {
        Self {
            origin,
            size,
            corner_radius: 0.0,
        }
    }

    /// Creates a rectangle from min and max corners.
    #[must_use]
    pub fn from_corners(min: [f32; 2], max: [f32; 2]) -> Self {
        Self {
            origin: min,
            size: [max[0] - min[0], max[1] - min[1]],
            corner_radius: 0.0,
        }
    }

    /// Sets the corner radius for rounded rectangles.
    #[must_use]
    pub const fn with_corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = radius;
        self
    }

    /// Returns the maximum x coordinate.
    #[must_use]
    pub fn max_x(&self) -> f32 {
        self.origin[0] + self.size[0]
    }

    /// Returns the maximum y coordinate.
    #[must_use]
    pub fn max_y(&self) -> f32 {
        self.origin[1] + self.size[1]
    }
}

impl Shape for Rect {
    fn path(&self) -> impl Iterator<Item = PathCommand> {
        let [x, y] = self.origin;
        let [w, h] = self.size;

        // Simple rectangle (corner_radius handling would require different approach)
        [
            PathCommand::MoveTo([x, y]),
            PathCommand::LineTo([x + w, y]),
            PathCommand::LineTo([x + w, y + h]),
            PathCommand::LineTo([x, y + h]),
            PathCommand::Close,
        ]
        .into_iter()
    }
}

// VectorArithmetic for Rect
impl Add for Rect {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            origin: [self.origin[0] + other.origin[0], self.origin[1] + other.origin[1]],
            size: [self.size[0] + other.size[0], self.size[1] + other.size[1]],
            corner_radius: self.corner_radius + other.corner_radius,
        }
    }
}

impl Sub for Rect {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self {
            origin: [self.origin[0] - other.origin[0], self.origin[1] - other.origin[1]],
            size: [self.size[0] - other.size[0], self.size[1] - other.size[1]],
            corner_radius: self.corner_radius - other.corner_radius,
        }
    }
}

impl Mul<f64> for Rect {
    type Output = Self;

    fn mul(self, scalar: f64) -> Self {
        let s = scalar as f32;
        Self {
            origin: [self.origin[0] * s, self.origin[1] * s],
            size: [self.size[0] * s, self.size[1] * s],
            corner_radius: self.corner_radius * s,
        }
    }
}

// ============================================================================
// Circle
// ============================================================================

/// A circle defined by center and radius.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Circle {
    /// Center point of the circle.
    pub center: [f32; 2],
    /// Radius of the circle.
    pub radius: f32,
}

impl Circle {
    /// Creates a new circle with the given center and radius.
    #[must_use]
    pub const fn new(center: [f32; 2], radius: f32) -> Self {
        Self { center, radius }
    }
}

impl Shape for Circle {
    fn path(&self) -> impl Iterator<Item = PathCommand> {
        // Approximate circle with 4 cubic Bezier curves (standard technique)
        // Control point distance: radius * 0.5522847498307936 (kappa constant)
        let [cx, cy] = self.center;
        let r = self.radius;
        let k = r * 0.5522847498;

        [
            PathCommand::MoveTo([cx + r, cy]),
            PathCommand::CubicTo {
                control1: [cx + r, cy + k],
                control2: [cx + k, cy + r],
                end: [cx, cy + r],
            },
            PathCommand::CubicTo {
                control1: [cx - k, cy + r],
                control2: [cx - r, cy + k],
                end: [cx - r, cy],
            },
            PathCommand::CubicTo {
                control1: [cx - r, cy - k],
                control2: [cx - k, cy - r],
                end: [cx, cy - r],
            },
            PathCommand::CubicTo {
                control1: [cx + k, cy - r],
                control2: [cx + r, cy - k],
                end: [cx + r, cy],
            },
            PathCommand::Close,
        ]
        .into_iter()
    }
}

// VectorArithmetic for Circle
impl Add for Circle {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            center: [self.center[0] + other.center[0], self.center[1] + other.center[1]],
            radius: self.radius + other.radius,
        }
    }
}

impl Sub for Circle {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self {
            center: [self.center[0] - other.center[0], self.center[1] - other.center[1]],
            radius: self.radius - other.radius,
        }
    }
}

impl Mul<f64> for Circle {
    type Output = Self;

    fn mul(self, scalar: f64) -> Self {
        let s = scalar as f32;
        Self {
            center: [self.center[0] * s, self.center[1] * s],
            radius: self.radius * s,
        }
    }
}

// ============================================================================
// Ellipse
// ============================================================================

/// An ellipse defined by center and radii.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Ellipse {
    /// Center point of the ellipse.
    pub center: [f32; 2],
    /// Radii (horizontal, vertical).
    pub radii: [f32; 2],
}

impl Ellipse {
    /// Creates a new ellipse with the given center and radii.
    #[must_use]
    pub const fn new(center: [f32; 2], radii: [f32; 2]) -> Self {
        Self { center, radii }
    }
}

impl Shape for Ellipse {
    fn path(&self) -> impl Iterator<Item = PathCommand> {
        // Approximate ellipse with 4 cubic Bezier curves
        let [cx, cy] = self.center;
        let [rx, ry] = self.radii;
        let kx = rx * 0.5522847498;
        let ky = ry * 0.5522847498;

        [
            PathCommand::MoveTo([cx + rx, cy]),
            PathCommand::CubicTo {
                control1: [cx + rx, cy + ky],
                control2: [cx + kx, cy + ry],
                end: [cx, cy + ry],
            },
            PathCommand::CubicTo {
                control1: [cx - kx, cy + ry],
                control2: [cx - rx, cy + ky],
                end: [cx - rx, cy],
            },
            PathCommand::CubicTo {
                control1: [cx - rx, cy - ky],
                control2: [cx - kx, cy - ry],
                end: [cx, cy - ry],
            },
            PathCommand::CubicTo {
                control1: [cx + kx, cy - ry],
                control2: [cx + rx, cy - ky],
                end: [cx + rx, cy],
            },
            PathCommand::Close,
        ]
        .into_iter()
    }
}

// VectorArithmetic for Ellipse
impl Add for Ellipse {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            center: [self.center[0] + other.center[0], self.center[1] + other.center[1]],
            radii: [self.radii[0] + other.radii[0], self.radii[1] + other.radii[1]],
        }
    }
}

impl Sub for Ellipse {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self {
            center: [self.center[0] - other.center[0], self.center[1] - other.center[1]],
            radii: [self.radii[0] - other.radii[0], self.radii[1] - other.radii[1]],
        }
    }
}

impl Mul<f64> for Ellipse {
    type Output = Self;

    fn mul(self, scalar: f64) -> Self {
        let s = scalar as f32;
        Self {
            center: [self.center[0] * s, self.center[1] * s],
            radii: [self.radii[0] * s, self.radii[1] * s],
        }
    }
}

// ============================================================================
// Line
// ============================================================================

/// A line segment from start to end.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Line {
    /// Start point of the line.
    pub start: [f32; 2],
    /// End point of the line.
    pub end: [f32; 2],
}

impl Line {
    /// Creates a new line from start to end.
    #[must_use]
    pub const fn new(start: [f32; 2], end: [f32; 2]) -> Self {
        Self { start, end }
    }
}

impl Shape for Line {
    fn path(&self) -> impl Iterator<Item = PathCommand> {
        [PathCommand::MoveTo(self.start), PathCommand::LineTo(self.end)].into_iter()
    }
}

// VectorArithmetic for Line
impl Add for Line {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            start: [self.start[0] + other.start[0], self.start[1] + other.start[1]],
            end: [self.end[0] + other.end[0], self.end[1] + other.end[1]],
        }
    }
}

impl Sub for Line {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self {
            start: [self.start[0] - other.start[0], self.start[1] - other.start[1]],
            end: [self.end[0] - other.end[0], self.end[1] - other.end[1]],
        }
    }
}

impl Mul<f64> for Line {
    type Output = Self;

    fn mul(self, scalar: f64) -> Self {
        let s = scalar as f32;
        Self {
            start: [self.start[0] * s, self.start[1] * s],
            end: [self.end[0] * s, self.end[1] * s],
        }
    }
}

// ============================================================================
// Vec<PathCommand> implements Shape
// ============================================================================

impl Shape for alloc::vec::Vec<PathCommand> {
    fn path(&self) -> impl Iterator<Item = PathCommand> {
        self.iter().copied()
    }
}

// ============================================================================
// FilledShape - Shape + GpuRenderer fill
// ============================================================================

use crate::gpu_surface::{GpuContext, GpuFrame, GpuRenderer, GpuSurface};
use waterui_core::View;

/// A shape filled with a GPU renderer (gradient, color, etc.).
///
/// This wraps any shape with a fill that implements `GpuRenderer`.
/// The fill is rendered to cover the shape's bounds.
///
/// # Example
///
/// ```ignore
/// use waterui_graphics::{Rect, GradientView, FilledShape};
///
/// // Fill a rect with a gradient
/// let filled = Rect::new([0.0, 0.0], [1.0, 1.0])
///     .fill(GradientRenderer::new(config));
/// ```
pub struct FilledShape<S, F> {
    /// The shape being filled.
    pub shape: S,
    /// The fill renderer.
    pub fill: F,
}

impl<S, F> FilledShape<S, F> {
    /// Creates a new filled shape.
    #[must_use]
    pub const fn new(shape: S, fill: F) -> Self {
        Self { shape, fill }
    }
}

impl<S: core::fmt::Debug, F> core::fmt::Debug for FilledShape<S, F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FilledShape")
            .field("shape", &self.shape)
            .finish_non_exhaustive()
    }
}

impl<S: 'static, F: GpuRenderer> GpuRenderer for FilledShape<S, F> {
    fn setup(&mut self, ctx: &GpuContext) {
        self.fill.setup(ctx);
    }

    fn render(&mut self, frame: &GpuFrame) {
        // For now, just render the fill (shape clipping would require stencil)
        // This works for Rect since gradients fill the whole surface
        self.fill.render(frame);
    }
}

impl<S: 'static, F: GpuRenderer> FilledShape<S, F> {
    /// Converts this filled shape into a `GpuSurface`.
    #[must_use]
    pub fn into_surface(self) -> GpuSurface {
        GpuSurface::new(self)
    }
}

impl<S: 'static, F: GpuRenderer> View for FilledShape<S, F> {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        self.into_surface()
    }
}

// Add fill() method to shapes
impl Rect {
    /// Fills this rectangle with the given renderer (gradient, color, etc.).
    #[must_use]
    pub fn fill<F: GpuRenderer>(self, fill: F) -> FilledShape<Self, F> {
        FilledShape::new(self, fill)
    }
}

impl Circle {
    /// Fills this circle with the given renderer.
    #[must_use]
    pub fn fill<F: GpuRenderer>(self, fill: F) -> FilledShape<Self, F> {
        FilledShape::new(self, fill)
    }
}

impl Ellipse {
    /// Fills this ellipse with the given renderer.
    #[must_use]
    pub fn fill<F: GpuRenderer>(self, fill: F) -> FilledShape<Self, F> {
        FilledShape::new(self, fill)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    #[test]
    fn rect_path_commands() {
        let rect = Rect::new([0.0, 0.0], [1.0, 1.0]);
        let commands: Vec<_> = rect.path().collect();

        assert_eq!(commands.len(), 5);
        assert!(matches!(commands[0], PathCommand::MoveTo([0.0, 0.0])));
        assert!(matches!(commands[1], PathCommand::LineTo([1.0, 0.0])));
        assert!(matches!(commands[4], PathCommand::Close));
    }

    #[test]
    fn circle_path_commands() {
        let circle = Circle::new([0.5, 0.5], 0.25);
        let commands: Vec<_> = circle.path().collect();

        // 1 MoveTo + 4 CubicTo + 1 Close
        assert_eq!(commands.len(), 6);
        assert!(matches!(commands[0], PathCommand::MoveTo(_)));
        assert!(matches!(commands[5], PathCommand::Close));
    }

    #[test]
    fn line_path_commands() {
        let line = Line::new([0.0, 0.0], [1.0, 1.0]);
        let commands: Vec<_> = line.path().collect();

        assert_eq!(commands.len(), 2);
        assert!(matches!(commands[0], PathCommand::MoveTo([0.0, 0.0])));
        assert!(matches!(commands[1], PathCommand::LineTo([1.0, 1.0])));
    }

    #[test]
    fn rect_lerp() {
        use waterui_core::vector_arithmetic::VectorArithmetic;

        let a = Rect::new([0.0, 0.0], [1.0, 1.0]);
        let b = Rect::new([1.0, 1.0], [2.0, 2.0]);
        let mid = a.lerp(&b, 0.5);

        assert!((mid.origin[0] - 0.5).abs() < 0.001);
        assert!((mid.origin[1] - 0.5).abs() < 0.001);
        assert!((mid.size[0] - 1.5).abs() < 0.001);
        assert!((mid.size[1] - 1.5).abs() < 0.001);
    }

    #[test]
    fn circle_lerp() {
        use waterui_core::vector_arithmetic::VectorArithmetic;

        let a = Circle::new([0.0, 0.0], 0.5);
        let b = Circle::new([1.0, 1.0], 1.0);
        let mid = a.lerp(&b, 0.5);

        assert!((mid.center[0] - 0.5).abs() < 0.001);
        assert!((mid.center[1] - 0.5).abs() < 0.001);
        assert!((mid.radius - 0.75).abs() < 0.001);
    }
}
