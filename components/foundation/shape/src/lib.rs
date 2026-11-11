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
const fn clamp_radius(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 0.5)
    } else {
        0.0
    }
}

#[derive(Debug, Clone, Copy)]
struct CornerRadii {
    top_left: f32,
    top_right: f32,
    bottom_right: f32,
    bottom_left: f32,
}

impl CornerRadii {
    #[inline]
    fn sanitized(mut self) -> Self {
        self.top_left = clamp_radius(self.top_left);
        self.top_right = clamp_radius(self.top_right);
        self.bottom_right = clamp_radius(self.bottom_right);
        self.bottom_left = clamp_radius(self.bottom_left);

        // Prevent overlapping corner arcs (same behavior as CSS border-radius normalization).
        let mut scale = 1.0f32;
        let pairs = [
            self.top_left + self.top_right,
            self.bottom_left + self.bottom_right,
            self.top_left + self.bottom_left,
            self.top_right + self.bottom_right,
        ];
        for sum in pairs {
            if sum > 1.0 {
                scale = scale.min(1.0 / sum);
            }
        }
        if scale < 1.0 {
            self.top_left *= scale;
            self.top_right *= scale;
            self.bottom_right *= scale;
            self.bottom_left *= scale;
        }
        self
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

/// A rectangle with uniform corner radius.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct RoundedRectangle {
    /// Corner radius (normalized, 0.0-0.5 range).
    corner_radius: f32,
}

impl RoundedRectangle {
    /// Corner radius (normalized, 0.0-0.5 range).
    #[must_use]
    pub const fn corner_radius(&self) -> f32 {
        self.corner_radius
    }
}

impl RoundedRectangle {
    /// Creates a new rounded rectangle with the given corner radius.
    ///
    /// The radius is **normalized**, not a length: it is a fraction of the
    /// shape's shorter side, so `0.5` is fully rounded and anything above that
    /// saturates there. Passing a point value (`28.0` for a 56pt-tall row)
    /// therefore lands on `0.5` rather than failing, which is only what was
    /// intended when the shape happens to be that tall.
    ///
    /// Reach for [`Capsule`] when the intent is "fully rounded at whatever size
    /// this ends up": it says so directly and cannot drift as the shape resizes.
    #[must_use]
    pub const fn new(corner_radius: f32) -> Self {
        Self { corner_radius }
    }
}

impl Shape for RoundedRectangle {
    type Iter = [PathCommand; 10];

    fn path(&self) -> Self::Iter {
        let r = CornerRadii {
            top_left: self.corner_radius,
            top_right: self.corner_radius,
            bottom_right: self.corner_radius,
            bottom_left: self.corner_radius,
        }
        .sanitized()
        .top_left;
        [
            PathCommand::MoveTo { x: r, y: 0.0 },
            PathCommand::LineTo { x: 1.0 - r, y: 0.0 },
            PathCommand::Arc {
                cx: 1.0 - r,
                cy: r,
                rx: r,
                ry: r,
                start: -FRAC_PI_2,
                sweep: FRAC_PI_2,
            },
            PathCommand::LineTo { x: 1.0, y: 1.0 - r },
            PathCommand::Arc {
                cx: 1.0 - r,
                cy: 1.0 - r,
                rx: r,
                ry: r,
                start: 0.0,
                sweep: FRAC_PI_2,
            },
            PathCommand::LineTo { x: r, y: 1.0 },
            PathCommand::Arc {
                cx: r,
                cy: 1.0 - r,
                rx: r,
                ry: r,
                start: FRAC_PI_2,
                sweep: FRAC_PI_2,
            },
            PathCommand::LineTo { x: 0.0, y: r },
            PathCommand::Arc {
                cx: r,
                cy: r,
                rx: r,
                ry: r,
                start: PI,
                sweep: FRAC_PI_2,
            },
            PathCommand::Close,
        ]
    }

    fn shape_kind(&self) -> ShapeKind {
        let r = CornerRadii {
            top_left: self.corner_radius,
            top_right: self.corner_radius,
            bottom_right: self.corner_radius,
            bottom_left: self.corner_radius,
        }
        .sanitized()
        .top_left;
        ShapeKind::RoundedRect { corner_radius: r }
    }
}

/// A rectangle with independent corner radii.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct UnevenRoundedRectangle {
    /// Top-leading corner radius (normalized).
    top_leading: f32,
    /// Top-trailing corner radius (normalized).
    top_trailing: f32,
    /// Bottom-leading corner radius (normalized).
    bottom_leading: f32,
    /// Bottom-trailing corner radius (normalized).
    bottom_trailing: f32,
}

impl UnevenRoundedRectangle {
    /// Top-leading corner radius (normalized).
    #[must_use]
    pub const fn top_leading(&self) -> f32 {
        self.top_leading
    }

    /// Top-trailing corner radius (normalized).
    #[must_use]
    pub const fn top_trailing(&self) -> f32 {
        self.top_trailing
    }

    /// Bottom-leading corner radius (normalized).
    #[must_use]
    pub const fn bottom_leading(&self) -> f32 {
        self.bottom_leading
    }

    /// Bottom-trailing corner radius (normalized).
    #[must_use]
    pub const fn bottom_trailing(&self) -> f32 {
        self.bottom_trailing
    }
}

impl UnevenRoundedRectangle {
    /// Creates a new uneven rounded rectangle with independent corner radii.
    #[must_use]
    pub const fn new(
        top_leading: f32,
        top_trailing: f32,
        bottom_leading: f32,
        bottom_trailing: f32,
    ) -> Self {
        Self {
            top_leading,
            top_trailing,
            bottom_leading,
            bottom_trailing,
        }
    }
}

impl Shape for UnevenRoundedRectangle {
    type Iter = [PathCommand; 10];

    fn path(&self) -> Self::Iter {
        let corners = CornerRadii {
            top_left: self.top_leading,
            top_right: self.top_trailing,
            bottom_right: self.bottom_trailing,
            bottom_left: self.bottom_leading,
        }
        .sanitized();
        let tl = corners.top_left;
        let tr = corners.top_right;
        let bl = corners.bottom_left;
        let br = corners.bottom_right;
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
        let corners = CornerRadii {
            top_left: self.top_leading,
            top_right: self.top_trailing,
            bottom_right: self.bottom_trailing,
            bottom_left: self.bottom_leading,
        }
        .sanitized();
        ShapeKind::UnevenRoundedRect {
            top_left: corners.top_left,
            top_right: corners.top_right,
            bottom_left: corners.bottom_left,
            bottom_right: corners.bottom_right,
        }
    }
}

/// A rectangle with a uniform corner radius in logical points.
///
/// [`RoundedRectangle`] expresses the corner as a fraction of the shorter side;
/// this type expresses it as an absolute length — the shape specs give for a
/// dialog (28dp) or a card (12dp). The rendered radius does not change as the
/// shape resizes, which is the whole point: spec radii stay constant however
/// tall or wide the surface ends up.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct FixedRoundedRectangle {
    /// Corner radius in logical points.
    corner_radius: f32,
}

impl FixedRoundedRectangle {
    /// Corner radius in logical points.
    #[must_use]
    pub const fn corner_radius(&self) -> f32 {
        self.corner_radius
    }
}

impl FixedRoundedRectangle {
    /// Creates a rounded rectangle whose corners are `corner_radius` points.
    ///
    /// The radius is clamped to half the shorter side when the shape resolves
    /// against its bounds — a radius wider than the surface degenerates to the
    /// capsule, the same way a normalized `0.5` does.
    #[must_use]
    pub const fn new(corner_radius: f32) -> Self {
        Self {
            corner_radius: if corner_radius.is_finite() {
                corner_radius.max(0.0)
            } else {
                0.0
            },
        }
    }
}

impl Shape for FixedRoundedRectangle {
    type Iter = [PathCommand; 10];

    /// Unit-space approximation only — maximally rounded, like a stadium.
    ///
    /// An absolute radius cannot be expressed in normalized commands without
    /// knowing the bounds. Backends must render this shape from
    /// [`ShapeKind::FixedRoundedRect`], not from these commands.
    fn path(&self) -> Self::Iter {
        RoundedRectangle::new(0.5).path()
    }

    fn shape_kind(&self) -> ShapeKind {
        ShapeKind::FixedRoundedRect {
            corner_radius: self.corner_radius,
        }
    }
}

/// A rectangle with independent corner radii in logical points.
///
/// The absolute-radius counterpart of [`UnevenRoundedRectangle`]: each corner
/// is a length in points, used where a spec names per-corner values — a modal
/// panel's trailing corners, say.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct FixedUnevenRoundedRectangle {
    /// Top-leading corner radius in logical points.
    top_leading: f32,
    /// Top-trailing corner radius in logical points.
    top_trailing: f32,
    /// Bottom-leading corner radius in logical points.
    bottom_leading: f32,
    /// Bottom-trailing corner radius in logical points.
    bottom_trailing: f32,
}

impl FixedUnevenRoundedRectangle {
    /// Top-leading corner radius in logical points.
    #[must_use]
    pub const fn top_leading(&self) -> f32 {
        self.top_leading
    }

    /// Top-trailing corner radius in logical points.
    #[must_use]
    pub const fn top_trailing(&self) -> f32 {
        self.top_trailing
    }

    /// Bottom-leading corner radius in logical points.
    #[must_use]
    pub const fn bottom_leading(&self) -> f32 {
        self.bottom_leading
    }

    /// Bottom-trailing corner radius in logical points.
    #[must_use]
    pub const fn bottom_trailing(&self) -> f32 {
        self.bottom_trailing
    }
}

impl FixedUnevenRoundedRectangle {
    /// Creates an uneven rounded rectangle with per-corner radii in points.
    #[must_use]
    pub const fn new(
        top_leading: f32,
        top_trailing: f32,
        bottom_leading: f32,
        bottom_trailing: f32,
    ) -> Self {
        const fn point_radius(radius: f32) -> f32 {
            if radius.is_finite() {
                radius.max(0.0)
            } else {
                0.0
            }
        }
        Self {
            top_leading: point_radius(top_leading),
            top_trailing: point_radius(top_trailing),
            bottom_leading: point_radius(bottom_leading),
            bottom_trailing: point_radius(bottom_trailing),
        }
    }
}

impl Shape for FixedUnevenRoundedRectangle {
    type Iter = [PathCommand; 10];

    /// Unit-space approximation only — each corner saturates independently,
    /// like [`UnevenRoundedRectangle`] at its maximum.
    ///
    /// Absolute radii cannot be expressed in normalized commands without
    /// knowing the bounds. Backends must render this shape from
    /// [`ShapeKind::FixedUnevenRoundedRect`], not from these commands.
    fn path(&self) -> Self::Iter {
        UnevenRoundedRectangle::new(
            clamp_radius(self.top_leading),
            clamp_radius(self.top_trailing),
            clamp_radius(self.bottom_leading),
            clamp_radius(self.bottom_trailing),
        )
        .path()
    }

    fn shape_kind(&self) -> ShapeKind {
        ShapeKind::FixedUnevenRoundedRect {
            top_left: self.top_leading,
            top_right: self.top_trailing,
            bottom_left: self.bottom_leading,
            bottom_right: self.bottom_trailing,
        }
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
    /// Rectangle with uniform corner radius.
    RoundedRect {
        /// Corner radius (normalized 0.0-0.5).
        corner_radius: f32,
    },
    /// Rectangle with per-corner radii.
    UnevenRoundedRect {
        /// Top-left corner radius.
        top_left: f32,
        /// Top-right corner radius.
        top_right: f32,
        /// Bottom-left corner radius.
        bottom_left: f32,
        /// Bottom-right corner radius.
        bottom_right: f32,
    },
    /// Capsule (pill) shape.
    Capsule,
    /// Rectangle with a uniform corner radius in logical points.
    ///
    /// Unlike [`ShapeKind::RoundedRect`], the radius is an absolute length, not
    /// a fraction of the shorter side: a `corner_radius` of `28.0` is 28 points
    /// whether the bounds are 280x140 or 560x300. Backends clamp it to half the
    /// shorter side at resolve time.
    FixedRoundedRect {
        /// Corner radius in logical points.
        corner_radius: f32,
    },
    /// Rectangle with per-corner radii in logical points.
    ///
    /// Same absolute semantics as [`ShapeKind::FixedRoundedRect`], with each
    /// corner named independently.
    FixedUnevenRoundedRect {
        /// Top-left corner radius in logical points.
        top_left: f32,
        /// Top-right corner radius in logical points.
        top_right: f32,
        /// Bottom-left corner radius in logical points.
        bottom_left: f32,
        /// Bottom-right corner radius in logical points.
        bottom_right: f32,
    },
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
pub struct FilledShape {
    kind: ShapeKind,
    commands: Vec<PathCommand>,
    fill: Color,
}

impl FilledShape {
    /// Creates a new filled shape from a shape and color.
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(shape: impl Shape, fill: impl Into<Color>) -> Self {
        Self {
            kind: ShapeKind::CustomPath,
            commands: shape.path().into_iter().collect(),
            fill: fill.into(),
        }
    }

    #[allow(clippy::needless_pass_by_value)]
    fn with_kind(kind: ShapeKind, shape: impl Shape, fill: impl Into<Color>) -> Self {
        Self {
            kind,
            commands: shape.path().into_iter().collect(),
            fill: fill.into(),
        }
    }

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

    /// Creates a morphing shape animation from this shape to another built-in shape.
    ///
    /// Morphing supports the built-in shapes, each a parameterized rounded
    /// rectangle; a [`Path`] has no parameters to interpolate.
    ///
    /// # Panics
    ///
    /// When either shape is a custom [`Path`].
    #[must_use]
    #[allow(clippy::needless_pass_by_value)]
    pub fn morph_to(self, target: impl ShapeExt) -> MorphShape {
        MorphShape::new(self.kind, target.shape_kind(), self.fill)
    }
}

impl View for FilledShape {
    fn body(self, env: &Environment) -> impl View {
        ResolvedShape {
            kind: self.kind,
            commands: self.commands,
            fill: self.fill.resolve(env).computed(),
        }
    }

    /// Resolves to `ResolvedShape`, which fills both axes.
    fn stretch_axis(&self) -> waterui_core::layout::StretchAxis {
        waterui_core::layout::StretchAxis::Both
    }
}

// ============================================================================
// ShapeExt - Extension trait for adding fill to shapes
// ============================================================================

/// Extension trait for filling shapes with color.
pub trait ShapeExt: Shape + Sized {
    /// Fills the shape with the specified color.
    fn fill(self, color: impl Into<Color>) -> FilledShape {
        FilledShape::with_kind(self.shape_kind(), self, color)
    }

    /// Creates a morphing filled shape from this shape to another built-in shape.
    ///
    /// Morphing supports the built-in shapes, each a parameterized rounded
    /// rectangle; a [`Path`] has no parameters to interpolate.
    ///
    /// # Panics
    ///
    /// When either shape is a custom [`Path`].
    fn morph_to(self, target: impl ShapeExt, fill: impl Into<Color>) -> MorphShape {
        MorphShape::new(self.shape_kind(), target.shape_kind(), fill.into())
    }
}

impl ShapeExt for Circle {}

impl ShapeExt for Ellipse {}

impl ShapeExt for Capsule {}

impl ShapeExt for Rectangle {}

impl ShapeExt for RoundedRectangle {}

impl ShapeExt for UnevenRoundedRectangle {}

impl ShapeExt for FixedRoundedRectangle {}

impl ShapeExt for FixedUnevenRoundedRectangle {}

impl ShapeExt for Path {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounded_rectangle_radius_is_clamped() {
        let kind = RoundedRectangle::new(9.0).shape_kind();
        match kind {
            ShapeKind::RoundedRect { corner_radius } => {
                assert!((corner_radius - 0.5).abs() < 1e-6);
            }
            _ => panic!("unexpected kind"),
        }
    }

    #[test]
    fn fixed_rounded_rectangle_carries_its_radius_in_points() {
        let kind = FixedRoundedRectangle::new(28.0).shape_kind();
        match kind {
            ShapeKind::FixedRoundedRect { corner_radius } => {
                assert!((corner_radius - 28.0).abs() < 1e-6);
            }
            _ => panic!("unexpected kind"),
        }
    }

    #[test]
    fn fixed_uneven_rounded_rectangle_carries_each_corner_in_points() {
        let kind = FixedUnevenRoundedRectangle::new(0.0, 16.0, 0.0, 16.0).shape_kind();
        match kind {
            ShapeKind::FixedUnevenRoundedRect {
                top_left,
                top_right,
                bottom_left,
                bottom_right,
            } => {
                assert!((top_left - 0.0).abs() < 1e-6);
                assert!((top_right - 16.0).abs() < 1e-6);
                assert!((bottom_left - 0.0).abs() < 1e-6);
                assert!((bottom_right - 16.0).abs() < 1e-6);
            }
            _ => panic!("unexpected kind"),
        }
    }

    #[test]
    fn fixed_radii_reject_negative_and_non_finite_values() {
        let kind = FixedRoundedRectangle::new(f32::NAN).shape_kind();
        match kind {
            ShapeKind::FixedRoundedRect { corner_radius } => {
                assert!((corner_radius - 0.0).abs() < 1e-6);
            }
            _ => panic!("unexpected kind"),
        }
        let kind = FixedRoundedRectangle::new(-4.0).shape_kind();
        match kind {
            ShapeKind::FixedRoundedRect { corner_radius } => {
                assert!((corner_radius - 0.0).abs() < 1e-6);
            }
            _ => panic!("unexpected kind"),
        }
    }

    #[test]
    fn uneven_radii_are_normalized_when_edges_overlap() {
        let kind = UnevenRoundedRectangle::new(0.8, 0.8, 0.8, 0.8).shape_kind();
        match kind {
            ShapeKind::UnevenRoundedRect {
                top_left,
                top_right,
                bottom_left,
                bottom_right,
            } => {
                assert!((top_left - 0.5).abs() < 1e-6);
                assert!((top_right - 0.5).abs() < 1e-6);
                assert!((bottom_left - 0.5).abs() < 1e-6);
                assert!((bottom_right - 0.5).abs() < 1e-6);
            }
            _ => panic!("unexpected kind"),
        }
    }
}
