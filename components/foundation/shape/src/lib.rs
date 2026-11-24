//! Shape system for `WaterUI` with HDR support.
//!
//! This module provides a trait-based system for defining shapes that can be used
//! for clipping views and as filled views.
//!
//! Filled shapes are emitted as native `ResolvedShape` raw views so each backend
//! renders paths with its own 2D engine. Morphing shapes interpolate the
//! built-in shapes' parameters and draw the resulting path through Canvas.
//!
//! # Example
//!
//! ```rust,ignore
//! use waterui::prelude::*;
//! use waterui::shape::*;
//!
//! // Clip to a circle
//! image("avatar.jpg").clip(Circle);
//!
//! // Fill a shape with HDR color
//! Circle.fill(Color::red().with_headroom(0.5))
//! ```

extern crate alloc;

use core::f32::consts::{FRAC_PI_2, PI, TAU};
use core::marker::PhantomData;

use nami::{Computed, SignalExt as _};
use waterui_core::{Environment, View, metadata::MetadataKey};
use waterui_graphics::color::Color;

mod morph;

pub use morph::{MorphAnimation, MorphShape};

// ============================================================================
// PathCommand - The primitive operations for drawing paths
// ============================================================================

/// A single path command for drawing shapes.
///
/// All coordinates are normalized (0.0-1.0) and scale with view bounds.
/// Native backends convert these to absolute coordinates based on view size.
#[derive(Debug, Clone, Copy, PartialEq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each path command variant by variant; a new path command must fail their build instead of falling into a wildcard arm"
)]
pub enum PathCommand {
    /// Move to a position without drawing.
    MoveTo {
        /// X coordinate (normalized 0.0-1.0)
        x: f32,
        /// Y coordinate (normalized 0.0-1.0)
        y: f32,
    },

    /// Draw a straight line to a position.
    LineTo {
        /// X coordinate (normalized 0.0-1.0)
        x: f32,
        /// Y coordinate (normalized 0.0-1.0)
        y: f32,
    },

    /// Draw a quadratic bezier curve.
    QuadTo {
        /// Control point x
        cx: f32,
        /// Control point y
        cy: f32,
        /// End point x
        x: f32,
        /// End point y
        y: f32,
    },

    /// Draw a cubic bezier curve.
    CubicTo {
        /// First control point x
        c1x: f32,
        /// First control point y
        c1y: f32,
        /// Second control point x
        c2x: f32,
        /// Second control point y
        c2y: f32,
        /// End point x
        x: f32,
        /// End point y
        y: f32,
    },

    /// Draw an arc.
    Arc {
        /// Center x (normalized)
        cx: f32,
        /// Center y (normalized)
        cy: f32,
        /// Radius x (normalized, relative to width)
        rx: f32,
        /// Radius y (normalized, relative to height)
        ry: f32,
        /// Start angle in radians
        start: f32,
        /// Sweep angle in radians (positive = clockwise)
        sweep: f32,
    },

    /// Close the current subpath by drawing a line to the start.
    Close,
}

#[inline]
const fn sanitize_fraction(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 0.5)
    } else {
        0.0
    }
}

#[inline]
const fn sanitize_points(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum CornerRadiusKind {
    Points(f32),
    Fraction(f32),
}

/// A corner radius for a [`RoundedRectangle`].
///
/// An `f32` converts to [`CornerRadius::points`]; use [`CornerRadius::fraction`]
/// when the radius should scale as a fraction of the shape's shorter side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CornerRadius {
    kind: CornerRadiusKind,
}

impl CornerRadius {
    /// Creates a corner radius in logical points.
    ///
    /// Negative or non-finite values sanitize to `0.0`. When resolved against
    /// bounds, the radius clamps to half the shape's shorter side.
    #[must_use]
    pub const fn points(points: f32) -> Self {
        Self {
            kind: CornerRadiusKind::Points(sanitize_points(points)),
        }
    }

    /// Creates a normalized corner radius as a fraction of the shorter side.
    ///
    /// Clamped to `0.0..=0.5`, where `0.5` is fully rounded (capsule caps);
    /// non-finite values sanitize to `0.0`.
    #[must_use]
    pub const fn fraction(fraction: f32) -> Self {
        Self {
            kind: CornerRadiusKind::Fraction(sanitize_fraction(fraction)),
        }
    }

    /// Resolves this radius to logical points against a shape's `shorter_side`.
    ///
    /// Both point and fraction radii clamp to `shorter_side / 2.0`.
    #[must_use]
    pub const fn resolve(self, shorter_side: f32) -> f32 {
        let side = sanitize_points(shorter_side);
        let limit = side / 2.0;
        match self.kind {
            CornerRadiusKind::Points(points) => points.min(limit),
            CornerRadiusKind::Fraction(fraction) => (fraction * side).min(limit),
        }
    }

    /// Returns the radius in logical points if this corner uses point units.
    #[must_use]
    pub const fn as_points(self) -> Option<f32> {
        match self.kind {
            CornerRadiusKind::Points(points) => Some(points),
            CornerRadiusKind::Fraction(_) => None,
        }
    }

    /// Returns the normalized fraction if this corner uses fractional units.
    #[must_use]
    pub const fn as_fraction(self) -> Option<f32> {
        match self.kind {
            CornerRadiusKind::Points(_) => None,
            CornerRadiusKind::Fraction(fraction) => Some(fraction),
        }
    }

    #[inline]
    const fn equals(self, other: Self) -> bool {
        match (self.kind, other.kind) {
            (CornerRadiusKind::Points(a), CornerRadiusKind::Points(b))
            | (CornerRadiusKind::Fraction(a), CornerRadiusKind::Fraction(b)) => {
                a.to_bits() == b.to_bits()
            }
            _ => false,
        }
    }

    #[inline]
    const fn unit_space(self) -> f32 {
        match self.kind {
            CornerRadiusKind::Points(points) => sanitize_fraction(points),
            CornerRadiusKind::Fraction(fraction) => fraction,
        }
    }
}

impl Default for CornerRadius {
    fn default() -> Self {
        Self::points(0.0)
    }
}

impl From<f32> for CornerRadius {
    fn from(points: f32) -> Self {
        Self::points(points)
    }
}

/// Per-corner radii for a [`RoundedRectangle`].
///
/// Corners are ordered `(top_leading, top_trailing, bottom_leading, bottom_trailing)`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CornerRadii {
    top_leading: CornerRadius,
    top_trailing: CornerRadius,
    bottom_leading: CornerRadius,
    bottom_trailing: CornerRadius,
}

impl CornerRadii {
    /// Creates per-corner radii from values convertible to [`CornerRadius`].
    #[must_use]
    pub fn new(
        top_leading: impl Into<CornerRadius>,
        top_trailing: impl Into<CornerRadius>,
        bottom_leading: impl Into<CornerRadius>,
        bottom_trailing: impl Into<CornerRadius>,
    ) -> Self {
        Self {
            top_leading: top_leading.into(),
            top_trailing: top_trailing.into(),
            bottom_leading: bottom_leading.into(),
            bottom_trailing: bottom_trailing.into(),
        }
    }

    /// `const` constructor from explicit [`CornerRadius`] values.
    #[must_use]
    pub const fn from_corners(
        top_leading: CornerRadius,
        top_trailing: CornerRadius,
        bottom_leading: CornerRadius,
        bottom_trailing: CornerRadius,
    ) -> Self {
        Self {
            top_leading,
            top_trailing,
            bottom_leading,
            bottom_trailing,
        }
    }

    /// Creates uniform radii across all four corners.
    #[must_use]
    pub const fn uniform(radius: CornerRadius) -> Self {
        Self {
            top_leading: radius,
            top_trailing: radius,
            bottom_leading: radius,
            bottom_trailing: radius,
        }
    }

    /// Top-leading corner radius.
    #[must_use]
    pub const fn top_leading(&self) -> CornerRadius {
        self.top_leading
    }

    /// Top-trailing corner radius.
    #[must_use]
    pub const fn top_trailing(&self) -> CornerRadius {
        self.top_trailing
    }

    /// Bottom-leading corner radius.
    #[must_use]
    pub const fn bottom_leading(&self) -> CornerRadius {
        self.bottom_leading
    }

    /// Bottom-trailing corner radius.
    #[must_use]
    pub const fn bottom_trailing(&self) -> CornerRadius {
        self.bottom_trailing
    }

    /// Returns the single radius if all four corners have identical values and units.
    #[must_use]
    pub const fn as_uniform(&self) -> Option<CornerRadius> {
        if self.top_leading.equals(self.top_trailing)
            && self.top_leading.equals(self.bottom_leading)
            && self.top_leading.equals(self.bottom_trailing)
        {
            Some(self.top_leading)
        } else {
            None
        }
    }

    /// Resolves all four corners to logical points in clockwise order:
    /// `[top_leading, top_trailing, bottom_trailing, bottom_leading]`.
    #[must_use]
    pub const fn resolve_clockwise(self, shorter_side: f32) -> [f32; 4] {
        [
            self.top_leading.resolve(shorter_side),
            self.top_trailing.resolve(shorter_side),
            self.bottom_trailing.resolve(shorter_side),
            self.bottom_leading.resolve(shorter_side),
        ]
    }

    /// Unit-space `[tl, tr, br, bl]` radii normalized against overlap.
    #[inline]
    fn unit_space_clockwise(self) -> [f32; 4] {
        let mut tl = self.top_leading.unit_space();
        let mut tr = self.top_trailing.unit_space();
        let mut br = self.bottom_trailing.unit_space();
        let mut bl = self.bottom_leading.unit_space();

        // Prevent overlapping corner arcs (same behavior as CSS border-radius normalization).
        let mut scale = 1.0f32;
        for sum in [tl + tr, bl + br, tl + bl, tr + br] {
            if sum > 1.0 {
                scale = scale.min(1.0 / sum);
            }
        }
        if scale < 1.0 {
            tl *= scale;
            tr *= scale;
            br *= scale;
            bl *= scale;
        }
        [tl, tr, br, bl]
    }
}

impl From<CornerRadius> for CornerRadii {
    fn from(radius: CornerRadius) -> Self {
        Self::uniform(radius)
    }
}

impl From<f32> for CornerRadii {
    fn from(points: f32) -> Self {
        Self::uniform(CornerRadius::points(points))
    }
}

// ============================================================================
// Shape Trait
// ============================================================================

/// A trait for types that can produce path commands for clipping.
///
/// All coordinates are normalized (0.0-1.0) and scale with view bounds.
/// Built-in shapes use stack-allocated arrays for zero heap allocation.
pub trait Shape {
    /// The iterator type returned by `path()`.
    type Iter: IntoIterator<Item = PathCommand>;

    /// Returns the path commands that define this shape.
    fn path(&self) -> Self::Iter;

    /// Returns what this shape *is*, for backends that can render it directly.
    ///
    /// Prefer this over [`Self::path`] wherever a backend can act on it. Path
    /// commands are normalized per axis, so resolving them against a non-square
    /// rect makes circular corners elliptical; the kind lets a backend resolve a
    /// normalized radius against the shorter side instead. Defaults to
    /// [`ShapeKind::CustomPath`], which means "only the path describes me".
    fn shape_kind(&self) -> ShapeKind {
        ShapeKind::CustomPath
    }
}

// ============================================================================
// Common Shape Implementations
// ============================================================================

/// A circle inscribed in the view bounds.
#[derive(Debug, Clone, Copy, Default)]
pub struct Circle;

impl Shape for Circle {
    type Iter = [PathCommand; 1];

    fn path(&self) -> Self::Iter {
        [PathCommand::Arc {
            cx: 0.5,
            cy: 0.5,
            rx: 0.5,
            ry: 0.5,
            start: 0.0,
            sweep: TAU,
        }]
    }

    fn shape_kind(&self) -> ShapeKind {
        ShapeKind::Circle
    }
}

/// An ellipse that fills the view bounds.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ellipse;

impl Shape for Ellipse {
    type Iter = [PathCommand; 1];

    fn path(&self) -> Self::Iter {
        [PathCommand::Arc {
            cx: 0.5,
            cy: 0.5,
            rx: 0.5,
            ry: 0.5,
            start: 0.0,
            sweep: TAU,
        }]
    }

    fn shape_kind(&self) -> ShapeKind {
        ShapeKind::Ellipse
    }
}

/// A capsule (pill) shape.
#[derive(Debug, Clone, Copy, Default)]
pub struct Capsule;

impl Shape for Capsule {
    type Iter = [PathCommand; 4];

    /// Unit-space approximation only — an ellipse inscribed in the box.
    ///
    /// A pill's caps are half its *shorter* side, which normalized per-axis
    /// coordinates cannot express without knowing the aspect ratio. Backends
    /// must render a capsule from [`ShapeKind::Capsule`], not from these
    /// commands.
    fn path(&self) -> Self::Iter {
        [
            PathCommand::MoveTo { x: 0.5, y: 0.0 },
            PathCommand::Arc {
                cx: 0.5,
                cy: 0.5,
                rx: 0.5,
                ry: 0.5,
                start: -FRAC_PI_2,
                sweep: PI,
            },
            PathCommand::Arc {
                cx: 0.5,
                cy: 0.5,
                rx: 0.5,
                ry: 0.5,
                start: FRAC_PI_2,
                sweep: PI,
            },
            PathCommand::Close,
        ]
    }

    fn shape_kind(&self) -> ShapeKind {
        ShapeKind::Capsule
    }
}

/// A rectangle with rounded corners.
///
/// Accepts a radius in logical points (`RoundedRectangle::new(12.0)`), a
/// [`CornerRadius`] (`CornerRadius::fraction(0.1)`), or per-corner
/// [`CornerRadii`]. Reach for [`Capsule`] when the intent is "fully rounded
/// at whatever size this ends up".
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[non_exhaustive]
pub struct RoundedRectangle {
    radii: CornerRadii,
}

impl RoundedRectangle {
    /// Creates a rounded rectangle from a uniform radius in points (`f32`), a
    /// [`CornerRadius`], or per-corner [`CornerRadii`].
    #[must_use]
    pub fn new(radii: impl Into<CornerRadii>) -> Self {
        Self {
            radii: radii.into(),
        }
    }

    /// `const` constructor from explicit [`CornerRadii`].
    #[must_use]
    pub const fn from_radii(radii: CornerRadii) -> Self {
        Self { radii }
    }

    /// Returns the corner radii configuration.
    #[must_use]
    pub const fn radii(&self) -> CornerRadii {
        self.radii
    }
}

impl Shape for RoundedRectangle {
    type Iter = [PathCommand; 10];

    /// Unit-space path commands.
    ///
    /// Point radii cannot be expressed exactly in normalized coordinates without
    /// knowing the bounds; backends must render this shape from
    /// [`ShapeKind::RoundedRect`], not from these commands.
    fn path(&self) -> Self::Iter {
        let [tl, tr, br, bl] = self.radii.unit_space_clockwise();
        [
            PathCommand::MoveTo { x: tl, y: 0.0 },
            PathCommand::LineTo {
                x: 1.0 - tr,
                y: 0.0,
            },
            PathCommand::Arc {
                cx: 1.0 - tr,
                cy: tr,
                rx: tr,
                ry: tr,
                start: -FRAC_PI_2,
                sweep: FRAC_PI_2,
            },
            PathCommand::LineTo {
                x: 1.0,
                y: 1.0 - br,
            },
            PathCommand::Arc {
                cx: 1.0 - br,
                cy: 1.0 - br,
                rx: br,
                ry: br,
                start: 0.0,
                sweep: FRAC_PI_2,
            },
            PathCommand::LineTo { x: bl, y: 1.0 },
            PathCommand::Arc {
                cx: bl,
                cy: 1.0 - bl,
                rx: bl,
                ry: bl,
                start: FRAC_PI_2,
                sweep: FRAC_PI_2,
            },
            PathCommand::LineTo { x: 0.0, y: tl },
            PathCommand::Arc {
                cx: tl,
                cy: tl,
                rx: tl,
                ry: tl,
                start: PI,
                sweep: FRAC_PI_2,
            },
            PathCommand::Close,
        ]
    }

    fn shape_kind(&self) -> ShapeKind {
        ShapeKind::RoundedRect { radii: self.radii }
    }
}

/// A simple rectangle with sharp corners.
#[derive(Debug, Clone, Copy, Default)]
pub struct Rectangle;

impl Shape for Rectangle {
    type Iter = [PathCommand; 5];

    fn path(&self) -> Self::Iter {
        [
            PathCommand::MoveTo { x: 0.0, y: 0.0 },
            PathCommand::LineTo { x: 1.0, y: 0.0 },
            PathCommand::LineTo { x: 1.0, y: 1.0 },
            PathCommand::LineTo { x: 0.0, y: 1.0 },
            PathCommand::Close,
        ]
    }

    fn shape_kind(&self) -> ShapeKind {
        ShapeKind::Rect
    }
}

// ============================================================================
// Custom Path Builder
// ============================================================================

/// A custom path defined by explicit commands.
#[derive(Debug, Clone, Default)]
pub struct Path {
    commands: Vec<PathCommand>,
}

impl Path {
    /// Creates a new empty path.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves to a position without drawing.
    #[must_use]
    pub fn move_to(mut self, x: f32, y: f32) -> Self {
        self.commands.push(PathCommand::MoveTo { x, y });
        self
    }

    /// Draws a straight line to a position.
    #[must_use]
    pub fn line_to(mut self, x: f32, y: f32) -> Self {
        self.commands.push(PathCommand::LineTo { x, y });
        self
    }

    /// Draws a quadratic bezier curve.
    #[must_use]
    pub fn quad_to(mut self, cx: f32, cy: f32, x: f32, y: f32) -> Self {
        self.commands.push(PathCommand::QuadTo { cx, cy, x, y });
        self
    }

    /// Draws a cubic bezier curve.
    #[must_use]
    pub fn cubic_to(mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) -> Self {
        self.commands.push(PathCommand::CubicTo {
            c1x,
            c1y,
            c2x,
            c2y,
            x,
            y,
        });
        self
    }

    /// Draws an arc.
    #[must_use]
    pub fn arc(mut self, cx: f32, cy: f32, rx: f32, ry: f32, start: f32, sweep: f32) -> Self {
        self.commands.push(PathCommand::Arc {
            cx,
            cy,
            rx,
            ry,
            start,
            sweep,
        });
        self
    }

    /// Closes the current subpath.
    #[must_use]
    pub fn close(mut self) -> Self {
        self.commands.push(PathCommand::Close);
        self
    }
}

impl Shape for Path {
    type Iter = alloc::vec::IntoIter<PathCommand>;

    fn path(&self) -> Self::Iter {
        self.commands.clone().into_iter()
    }

    fn shape_kind(&self) -> ShapeKind {
        ShapeKind::CustomPath
    }
}

// ============================================================================
// ClipShape Metadata
// ============================================================================

/// Metadata for clipping a view to a shape.
///
/// Carries both the structured [`ShapeKind`] and the unit-space path. Backends
/// should prefer the kind: [`PathCommand`] coordinates are normalized per axis,
/// so resolving them against a non-square rect turns a circular corner into an
/// elliptical one — a fully-rounded clip comes out as an ellipse instead of a
/// pill. The kind says what the shape *is*, letting a backend resolve a
/// normalized radius against the shorter side the way [`FilledShape`] already
/// does. The commands remain the fallback for [`ShapeKind::CustomPath`].
#[derive(Debug)]
pub struct ClipShape {
    kind: ShapeKind,
    commands: Vec<PathCommand>,
}

impl ClipShape {
    /// Creates a new clip shape from any type implementing Shape.
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(shape: impl Shape) -> Self {
        Self {
            kind: shape.shape_kind(),
            commands: shape.path().into_iter().collect(),
        }
    }

    /// Returns the structured shape kind. Prefer this over [`Self::commands`];
    /// see the type documentation.
    #[must_use]
    pub const fn kind(&self) -> ShapeKind {
        self.kind
    }

    /// Returns the unit-space path commands.
    #[must_use]
    pub fn commands(&self) -> &[PathCommand] {
        &self.commands
    }
}

impl MetadataKey for ClipShape {}

// ============================================================================
// ShapeKind - For backend rendering optimization
// ============================================================================

/// The kind of shape for backend rendering optimization.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends render each shape kind with its own native primitive, variant by variant; a new kind must fail their build instead of falling into a wildcard arm"
)]
pub enum ShapeKind {
    /// Rectangle with sharp corners.
    #[default]
    Rect,
    /// Circle inscribed in bounds.
    Circle,
    /// Ellipse filling bounds.
    Ellipse,
    /// Rectangle with rounded corners.
    RoundedRect {
        /// Per-corner radii configuration.
        radii: CornerRadii,
    },
    /// Capsule (pill) shape.
    Capsule,
    /// Custom path.
    CustomPath,
}

/// Resolved shape payload rendered directly by native backends.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ResolvedShape {
    /// Shape kind for backend-side optimization.
    kind: ShapeKind,
    /// Path commands in unit coordinate space.
    commands: Vec<PathCommand>,
    /// Environment-resolved fill color that remains reactive to theme changes.
    fill: Computed<waterui_graphics::WorkingColor>,
}

impl ResolvedShape {
    /// Shape kind for backend-side optimization.
    #[must_use]
    pub const fn kind(&self) -> ShapeKind {
        self.kind
    }

    /// Path commands in unit coordinate space.
    #[must_use]
    pub const fn commands(&self) -> &[PathCommand] {
        self.commands.as_slice()
    }

    /// Environment-resolved fill color that remains reactive to theme changes.
    #[must_use]
    pub const fn fill(&self) -> &Computed<waterui_graphics::WorkingColor> {
        &self.fill
    }
}

waterui_core::raw_view!(ResolvedShape, waterui_core::layout::StretchAxis::Both);

// ============================================================================
// FilledShape - Shape as a View with backend-native fill rendering
// ============================================================================

/// A shape filled with a color, resolved to `ResolvedShape`.
#[derive(Debug)]
pub struct FilledShape<S = ()> {
    kind: ShapeKind,
    commands: Vec<PathCommand>,
    fill: Color,
    _shape: PhantomData<fn() -> S>,
}

impl<S: Shape> FilledShape<S> {
    /// Creates a new filled shape from a shape and color.
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(shape: S, fill: impl Into<Color>) -> Self {
        Self {
            kind: shape.shape_kind(),
            commands: shape.path().into_iter().collect(),
            fill: fill.into(),
            _shape: PhantomData,
        }
    }
}

impl<S> FilledShape<S> {
    /// Returns the path commands.
    #[must_use]
    pub fn commands(&self) -> &[PathCommand] {
        &self.commands
    }

    /// Returns the fill color.
    #[must_use]
    pub const fn fill(&self) -> &Color {
        &self.fill
    }

    /// Returns the shape kind.
    #[must_use]
    pub const fn kind(&self) -> ShapeKind {
        self.kind
    }
}

impl<S: MorphableShape> FilledShape<S> {
    /// Creates a morphing shape animation from this filled shape to another
    /// [`MorphableShape`].
    ///
    /// Morphing supports only the built-in parameterized shapes; custom paths
    /// do not implement [`MorphableShape`] and fail to compile.
    ///
    /// ```compile_fail
    /// use waterui_graphics::color::Color;
    /// use waterui_shape::{Circle, Path, ShapeExt};
    ///
    /// let custom = Path::new().move_to(0.0, 0.0).line_to(1.0, 1.0).close();
    /// let _ = custom.fill(Color::srgb(255, 0, 0)).morph_to(Circle);
    /// ```
    #[must_use]
    #[allow(clippy::needless_pass_by_value)]
    pub fn morph_to(self, target: impl MorphableShape) -> MorphShape {
        MorphShape::new(self.kind, target.shape_kind(), self.fill)
    }
}

impl<S: 'static> View for FilledShape<S> {
    fn body(self, env: &Environment) -> impl View {
        ResolvedShape {
            kind: self.kind,
            commands: self.commands,
            fill: self.fill.resolve(env).computed(),
        }
    }
}

mod sealed {
    pub trait Sealed {}
}

/// Built-in parameterized shapes whose geometry can interpolate in [`MorphShape`].
///
/// Only the built-in shapes (`Circle`, `Ellipse`, `Capsule`, `Rectangle`, and
/// `RoundedRectangle`) implement this trait. Custom [`Path`] values
/// have no shared corner/box parameters and therefore cannot morph.
///
/// ```compile_fail
/// use waterui_graphics::color::Color;
/// use waterui_shape::{Circle, Path, ShapeExt};
///
/// let tri = Path::new()
///     .move_to(0.5, 0.0)
///     .line_to(1.0, 1.0)
///     .line_to(0.0, 1.0)
///     .close();
/// let _ = Circle.morph_to(tri, Color::srgb(255, 0, 0));
/// ```
///
/// ```compile_fail
/// use waterui_graphics::color::Color;
/// use waterui_shape::{Circle, Path, ShapeExt};
///
/// let tri = Path::new()
///     .move_to(0.5, 0.0)
///     .line_to(1.0, 1.0)
///     .line_to(0.0, 1.0)
///     .close();
/// let _ = tri.morph_to(Circle, Color::srgb(255, 0, 0));
/// ```
pub trait MorphableShape: ShapeExt + sealed::Sealed {}

// ============================================================================
// ShapeExt - Extension trait for adding fill to shapes
// ============================================================================

/// Extension trait for filling shapes with color.
pub trait ShapeExt: Shape + Sized {
    /// Fills the shape with the specified color.
    fn fill(self, color: impl Into<Color>) -> FilledShape<Self> {
        FilledShape::new(self, color)
    }

    /// Creates a morphing filled shape from this shape to another [`MorphableShape`].
    ///
    /// Morphing supports only the built-in parameterized shapes; custom paths
    /// do not implement [`MorphableShape`] and fail to compile.
    fn morph_to(self, target: impl MorphableShape, fill: impl Into<Color>) -> MorphShape
    where
        Self: MorphableShape,
    {
        MorphShape::new(self.shape_kind(), target.shape_kind(), fill.into())
    }
}

macro_rules! impl_morphable_shape {
    ($($ty:ty),* $(,)?) => {
        $(
            impl ShapeExt for $ty {}
            impl sealed::Sealed for $ty {}
            impl MorphableShape for $ty {}
        )*
    };
}

impl_morphable_shape!(Circle, Ellipse, Capsule, Rectangle, RoundedRectangle);

impl ShapeExt for Path {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filled_shape_new_keeps_kind() {
        let via_fill = Capsule.fill(Color::srgb(255, 0, 0));
        let via_new = FilledShape::new(Capsule, Color::srgb(255, 0, 0));
        assert!(matches!(via_fill.kind(), ShapeKind::Capsule));
        assert!(
            matches!(via_new.kind(), ShapeKind::Capsule),
            "FilledShape::new(Capsule, c) lost the kind: got CustomPath = {}",
            matches!(via_new.kind(), ShapeKind::CustomPath)
        );
    }

    #[test]
    fn rounded_rectangle_new_defaults_to_points() {
        let kind = RoundedRectangle::new(28.0).shape_kind();
        match kind {
            ShapeKind::RoundedRect { radii } => {
                assert_eq!(radii, CornerRadii::uniform(CornerRadius::points(28.0)));
                assert!((radii.resolve_clockwise(100.0)[0] - 28.0).abs() < 1e-6);
                assert!((radii.resolve_clockwise(40.0)[0] - 20.0).abs() < 1e-6);
            }
            _ => panic!("unexpected kind"),
        }
    }

    #[test]
    fn rounded_rectangle_fraction_scales_with_shorter_side_and_clamps() {
        let kind = RoundedRectangle::new(CornerRadius::fraction(9.0)).shape_kind();
        match kind {
            ShapeKind::RoundedRect { radii } => {
                assert_eq!(radii, CornerRadii::uniform(CornerRadius::fraction(0.5)));
                assert!((radii.resolve_clockwise(100.0)[0] - 50.0).abs() < 1e-6);
            }
            _ => panic!("unexpected kind"),
        }
    }

    #[test]
    fn rounded_rectangle_carries_per_corner_radii() {
        let kind = RoundedRectangle::new(CornerRadii::new(0.0, 16.0, 0.0, 16.0)).shape_kind();
        match kind {
            ShapeKind::RoundedRect { radii } => {
                assert_eq!(radii.resolve_clockwise(100.0), [0.0, 16.0, 16.0, 0.0]);
            }
            _ => panic!("unexpected kind"),
        }
    }

    #[test]
    fn point_radii_reject_negative_and_non_finite_values() {
        assert_eq!(CornerRadius::points(f32::NAN), CornerRadius::points(0.0));
        assert_eq!(CornerRadius::points(-4.0), CornerRadius::points(0.0));
        assert_eq!(
            CornerRadius::fraction(f32::NAN),
            CornerRadius::fraction(0.0)
        );
        assert_eq!(CornerRadius::fraction(-1.0), CornerRadius::fraction(0.0));
    }
}
