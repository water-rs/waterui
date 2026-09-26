//! Shapes for `WaterUI`: normalized descriptions that resolve to Cherenkov
//! [`ShapeData`] against a view's bounds.
//!
//! A [`Shape`] describes itself in the unit square, so it scales with the view
//! it clips or fills. A backend that renders through Cherenkov calls
//! [`Shape::resolve`] (or [`resolve_shape`] on the erased [`ShapeKind`] and
//! path) with the view's rect and records the result; native backends receive
//! the kind and the normalized path over the FFI and draw it themselves.
//!
//! Filled shapes are emitted as native `ResolvedShape` raw views so each
//! backend renders paths with its own 2D engine.
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
//! // Fill a shape
//! Circle.fill(Color::red())
//! ```

use core::f64::consts::{FRAC_PI_2, PI};
use core::time::Duration;

pub use cherenkov::kurbo;
use cherenkov::kurbo::{
    Affine, Arc, BezPath, Circle as CircleGeometry, Ellipse as EllipseGeometry, PathEl, Point,
    Rect, RoundedRect, RoundedRectRadii, Shape as _, Vec2,
};
use cherenkov::{Curve, WorkingColor};
pub use cherenkov::{FillRule, ShapeData};
use nami::{Computed, SignalExt as _, signal::IntoComputed};
use waterui_core::{Environment, View, metadata::MetadataKey};
use waterui_graphics::color::Color;

/// Flattening tolerance for arcs in the unit square.
const UNIT_TOLERANCE: f64 = 1e-4;

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

    /// The unit-square outline, corners as quarter arcs.
    fn unit_path(self) -> BezPath {
        let mut path = BezPath::new();
        let tl = f64::from(self.top_left);
        let tr = f64::from(self.top_right);
        let br = f64::from(self.bottom_right);
        let bl = f64::from(self.bottom_left);
        path.move_to((tl, 0.0));
        path.line_to((1.0 - tr, 0.0));
        append_arc(&mut path, (1.0 - tr, tr), tr, -FRAC_PI_2, FRAC_PI_2);
        path.line_to((1.0, 1.0 - br));
        append_arc(&mut path, (1.0 - br, 1.0 - br), br, 0.0, FRAC_PI_2);
        path.line_to((bl, 1.0));
        append_arc(&mut path, (bl, 1.0 - bl), bl, FRAC_PI_2, FRAC_PI_2);
        path.line_to((0.0, tl));
        append_arc(&mut path, (tl, tl), tl, PI, FRAC_PI_2);
        path.close_path();
        path
    }

    fn radii(self, scale: f64) -> RoundedRectRadii {
        RoundedRectRadii::new(
            f64::from(self.top_left) * scale,
            f64::from(self.top_right) * scale,
            f64::from(self.bottom_right) * scale,
            f64::from(self.bottom_left) * scale,
        )
    }
}

fn append_arc(path: &mut BezPath, center: (f64, f64), radius: f64, start: f64, sweep: f64) {
    if radius <= 0.0 {
        return;
    }
    let arc = Arc::new(center, (radius, radius), start, sweep, 0.0);
    path.extend(arc.append_iter(UNIT_TOLERANCE));
}

// ============================================================================
// Shape Trait
// ============================================================================

/// A shape described in the unit square, resolved against a view's bounds.
///
/// Built-in shapes carry a [`ShapeKind`] a backend can act on directly: path
/// coordinates are normalized per axis, so resolving them against a
/// non-square rect makes circular corners elliptical, while the kind resolves
/// a normalized radius against the shorter side instead.
pub trait Shape {
    /// The path in normalized (0.0–1.0) coordinates.
    fn path(&self) -> BezPath;

    /// What this shape *is*, for backends that can render it directly.
    ///
    /// Defaults to [`ShapeKind::CustomPath`]: only the path describes it.
    fn shape_kind(&self) -> ShapeKind {
        ShapeKind::CustomPath
    }

    /// The Cherenkov shape filling `bounds`.
    fn resolve(&self, bounds: Rect) -> ShapeData {
        resolve_shape(self.shape_kind(), &self.path(), bounds)
    }
}

/// Resolves an erased shape (its kind and normalized path) against `bounds`.
///
/// Built-in kinds become Cherenkov's semantic shapes, with normalized radii
/// measured against the shorter side and point radii clamped to half of it;
/// a custom path is scaled per axis into the rect.
#[must_use]
pub fn resolve_shape(kind: ShapeKind, path: &BezPath, bounds: Rect) -> ShapeData {
    let shorter = bounds.width().min(bounds.height());
    let point = |radius: f32| f64::from(radius).min(shorter / 2.0);
    match kind {
        ShapeKind::Rect => ShapeData::Rect(bounds),
        ShapeKind::Circle => ShapeData::Circle(CircleGeometry::new(bounds.center(), shorter / 2.0)),
        ShapeKind::Ellipse => ShapeData::Ellipse(EllipseGeometry::from_rect(bounds)),
        ShapeKind::RoundedRect { corner_radius } => ShapeData::RoundedRect(RoundedRect::from_rect(
            bounds,
            f64::from(clamp_radius(corner_radius)) * shorter,
        )),
        ShapeKind::UnevenRoundedRect {
            top_left,
            top_right,
            bottom_left,
            bottom_right,
        } => ShapeData::RoundedRect(RoundedRect::from_rect(
            bounds,
            CornerRadii {
                top_left,
                top_right,
                bottom_right,
                bottom_left,
            }
            .sanitized()
            .radii(shorter),
        )),
        ShapeKind::Capsule => ShapeData::RoundedRect(RoundedRect::from_rect(bounds, shorter / 2.0)),
        ShapeKind::FixedRoundedRect { corner_radius } => {
            ShapeData::RoundedRect(RoundedRect::from_rect(bounds, point(corner_radius)))
        }
        ShapeKind::FixedUnevenRoundedRect {
            top_left,
            top_right,
            bottom_left,
            bottom_right,
        } => ShapeData::RoundedRect(RoundedRect::from_rect(
            bounds,
            RoundedRectRadii::new(
                point(top_left),
                point(top_right),
                point(bottom_right),
                point(bottom_left),
            ),
        )),
        ShapeKind::CustomPath => {
            let transform = Affine::translate(bounds.origin().to_vec2())
                * Affine::scale_non_uniform(bounds.width(), bounds.height());
            ShapeData::Path {
                elements: path.elements().iter().map(|el| transform * *el).collect(),
                rule: FillRule::NonZero,
            }
        }
    }
}

// ============================================================================
// Common Shape Implementations
// ============================================================================

/// A circle inscribed in the view bounds.
#[derive(Debug, Clone, Copy, Default)]
pub struct Circle;

impl Shape for Circle {
    fn path(&self) -> BezPath {
        CircleGeometry::new((0.5, 0.5), 0.5).to_path(UNIT_TOLERANCE)
    }

    fn shape_kind(&self) -> ShapeKind {
        ShapeKind::Circle
    }
}

/// An ellipse filling the view bounds.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ellipse;

impl Shape for Ellipse {
    fn path(&self) -> BezPath {
        EllipseGeometry::new((0.5, 0.5), (0.5, 0.5), 0.0).to_path(UNIT_TOLERANCE)
    }

    fn shape_kind(&self) -> ShapeKind {
        ShapeKind::Ellipse
    }
}

/// A capsule (stadium): a rectangle with fully rounded ends.
#[derive(Debug, Clone, Copy, Default)]
pub struct Capsule;

impl Shape for Capsule {
    fn path(&self) -> BezPath {
        CornerRadii {
            top_left: 0.5,
            top_right: 0.5,
            bottom_right: 0.5,
            bottom_left: 0.5,
        }
        .unit_path()
    }

    fn shape_kind(&self) -> ShapeKind {
        ShapeKind::Capsule
    }
}

/// A rectangle with uniform corner radius.
#[derive(Debug, Clone, Copy)]
pub struct RoundedRectangle {
    /// Corner radius (normalized, 0.0-0.5 range).
    pub corner_radius: f32,
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
    fn path(&self) -> BezPath {
        CornerRadii {
            top_left: self.corner_radius,
            top_right: self.corner_radius,
            bottom_right: self.corner_radius,
            bottom_left: self.corner_radius,
        }
        .sanitized()
        .unit_path()
    }

    fn shape_kind(&self) -> ShapeKind {
        ShapeKind::RoundedRect {
            corner_radius: clamp_radius(self.corner_radius),
        }
    }
}

/// A rectangle with independent corner radii.
#[derive(Debug, Clone, Copy)]
pub struct UnevenRoundedRectangle {
    /// Top-leading corner radius (normalized).
    pub top_leading: f32,
    /// Top-trailing corner radius (normalized).
    pub top_trailing: f32,
    /// Bottom-leading corner radius (normalized).
    pub bottom_leading: f32,
    /// Bottom-trailing corner radius (normalized).
    pub bottom_trailing: f32,
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

    fn corners(&self) -> CornerRadii {
        CornerRadii {
            top_left: self.top_leading,
            top_right: self.top_trailing,
            bottom_right: self.bottom_trailing,
            bottom_left: self.bottom_leading,
        }
        .sanitized()
    }
}

impl Shape for UnevenRoundedRectangle {
    fn path(&self) -> BezPath {
        self.corners().unit_path()
    }

    fn shape_kind(&self) -> ShapeKind {
        let corners = self.corners();
        ShapeKind::UnevenRoundedRect {
            top_left: corners.top_left,
            top_right: corners.top_right,
            bottom_left: corners.bottom_left,
            bottom_right: corners.bottom_right,
        }
    }
}

/// A rectangle whose corner radius is a fixed length in logical points.
///
/// This is the shape a design system's spec radii describe: an M3 medium
/// dialog (28dp) or a card (12dp). The rendered radius does not change as the
/// shape resizes, which is the whole point: spec radii stay constant however
/// tall or wide the surface ends up.
#[derive(Debug, Clone, Copy)]
pub struct FixedRoundedRectangle {
    /// Corner radius in logical points.
    pub corner_radius: f32,
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
            corner_radius: point_radius(corner_radius),
        }
    }
}

impl Shape for FixedRoundedRectangle {
    /// Unit-space approximation only — maximally rounded, like a stadium.
    ///
    /// An absolute radius cannot be expressed in normalized coordinates
    /// without knowing the bounds. Backends must render this shape from
    /// [`ShapeKind::FixedRoundedRect`], not from this path.
    fn path(&self) -> BezPath {
        Capsule.path()
    }

    fn shape_kind(&self) -> ShapeKind {
        ShapeKind::FixedRoundedRect {
            corner_radius: self.corner_radius,
        }
    }
}

const fn point_radius(radius: f32) -> f32 {
    if radius.is_finite() {
        radius.max(0.0)
    } else {
        0.0
    }
}

/// A rectangle with independent corner radii in logical points.
///
/// The fixed-radius counterpart of [`UnevenRoundedRectangle`]: a bottom sheet
/// that rounds only its top corners, or a navigation drawer rounding its
/// panel's trailing corners, say.
#[derive(Debug, Clone, Copy)]
pub struct FixedUnevenRoundedRectangle {
    /// Top-leading corner radius in logical points.
    pub top_leading: f32,
    /// Top-trailing corner radius in logical points.
    pub top_trailing: f32,
    /// Bottom-leading corner radius in logical points.
    pub bottom_leading: f32,
    /// Bottom-trailing corner radius in logical points.
    pub bottom_trailing: f32,
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
        Self {
            top_leading: point_radius(top_leading),
            top_trailing: point_radius(top_trailing),
            bottom_leading: point_radius(bottom_leading),
            bottom_trailing: point_radius(bottom_trailing),
        }
    }
}

impl Shape for FixedUnevenRoundedRectangle {
    /// Unit-space approximation only — each corner saturates independently,
    /// like [`UnevenRoundedRectangle`] at its maximum.
    ///
    /// Absolute radii cannot be expressed in normalized coordinates without
    /// knowing the bounds. Backends must render this shape from
    /// [`ShapeKind::FixedUnevenRoundedRect`], not from this path.
    fn path(&self) -> BezPath {
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

/// A rectangle filling the view bounds.
#[derive(Debug, Clone, Copy, Default)]
pub struct Rectangle;

impl Shape for Rectangle {
    fn path(&self) -> BezPath {
        Rect::new(0.0, 0.0, 1.0, 1.0).to_path(UNIT_TOLERANCE)
    }

    fn shape_kind(&self) -> ShapeKind {
        ShapeKind::Rect
    }
}

/// A custom path built from normalized coordinates.
///
/// All coordinates are in the 0.0–1.0 range and scale with the view bounds.
#[derive(Debug, Clone, Default)]
pub struct Path {
    path: BezPath,
}

impl Path {
    /// Creates an empty path.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves to a position without drawing.
    #[must_use]
    pub fn move_to(mut self, x: f32, y: f32) -> Self {
        self.path.move_to(point(x, y));
        self
    }

    /// Draws a straight line to a position.
    #[must_use]
    pub fn line_to(mut self, x: f32, y: f32) -> Self {
        self.path.line_to(point(x, y));
        self
    }

    /// Draws a quadratic bezier curve.
    #[must_use]
    pub fn quad_to(mut self, cx: f32, cy: f32, x: f32, y: f32) -> Self {
        self.path.quad_to(point(cx, cy), point(x, y));
        self
    }

    /// Draws a cubic bezier curve.
    #[must_use]
    pub fn cubic_to(mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) -> Self {
        self.path
            .curve_to(point(c1x, c1y), point(c2x, c2y), point(x, y));
        self
    }

    /// Draws an elliptical arc around `(cx, cy)`, from `start` over `sweep`
    /// radians (positive = clockwise), connecting from the current point.
    #[must_use]
    pub fn arc(mut self, cx: f32, cy: f32, rx: f32, ry: f32, start: f32, sweep: f32) -> Self {
        let arc = Arc::new(
            point(cx, cy),
            Vec2::new(f64::from(rx), f64::from(ry)),
            f64::from(start),
            f64::from(sweep),
            0.0,
        );
        let mut elements = arc.path_elements(UNIT_TOLERANCE);
        match (self.path.elements().is_empty(), elements.next()) {
            (true, Some(PathEl::MoveTo(start))) => self.path.move_to(start),
            (false, Some(PathEl::MoveTo(start))) => self.path.line_to(start),
            (_, Some(other)) => self.path.push(other),
            (_, None) => {}
        }
        self.path.extend(elements);
        self
    }

    /// Closes the current subpath by drawing a line to its start.
    #[must_use]
    pub fn close(mut self) -> Self {
        self.path.close_path();
        self
    }
}

fn point(x: f32, y: f32) -> Point {
    Point::new(f64::from(x), f64::from(y))
}

impl Shape for Path {
    fn path(&self) -> BezPath {
        self.path.clone()
    }
}

// ============================================================================
// ClipShape Metadata
// ============================================================================

/// Metadata clipping a view to a shape.
#[derive(Debug)]
pub struct ClipShape {
    kind: ShapeKind,
    path: BezPath,
}

impl ClipShape {
    /// Clips to `shape`.
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(shape: impl Shape) -> Self {
        Self {
            kind: shape.shape_kind(),
            path: shape.path(),
        }
    }

    /// What the shape is, for backends that render kinds directly.
    #[must_use]
    pub const fn kind(&self) -> ShapeKind {
        self.kind
    }

    /// The normalized path.
    #[must_use]
    pub const fn path(&self) -> &BezPath {
        &self.path
    }

    /// The Cherenkov shape clipping `bounds`.
    #[must_use]
    pub fn resolve(&self, bounds: Rect) -> ShapeData {
        resolve_shape(self.kind, &self.path, bounds)
    }
}

impl MetadataKey for ClipShape {}

// ============================================================================
// ShapeKind - For backend rendering optimization
// ============================================================================

/// What a shape *is*, so a backend can render it from its parameters.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum ShapeKind {
    /// The full bounds.
    #[default]
    Rect,
    /// A circle inscribed in the bounds.
    Circle,
    /// An ellipse filling the bounds.
    Ellipse,
    /// A rectangle with one normalized radius (fraction of the shorter side).
    RoundedRect {
        /// Normalized corner radius, 0.0–0.5.
        corner_radius: f32,
    },
    /// A rectangle with per-corner normalized radii.
    UnevenRoundedRect {
        /// Top-left radius.
        top_left: f32,
        /// Top-right radius.
        top_right: f32,
        /// Bottom-left radius.
        bottom_left: f32,
        /// Bottom-right radius.
        bottom_right: f32,
    },
    /// A stadium: fully rounded ends.
    Capsule,
    /// A rectangle with one radius in logical points.
    FixedRoundedRect {
        /// Corner radius in points.
        corner_radius: f32,
    },
    /// A rectangle with per-corner radii in logical points.
    FixedUnevenRoundedRect {
        /// Top-left radius in points.
        top_left: f32,
        /// Top-right radius in points.
        top_right: f32,
        /// Bottom-left radius in points.
        bottom_left: f32,
        /// Bottom-right radius in points.
        bottom_right: f32,
    },
    /// Only the path describes the shape.
    CustomPath,
}

/// A filled shape as a backend receives it: kind, normalized path and the
/// environment-resolved fill.
#[derive(Debug, Clone)]
pub struct ResolvedShape {
    /// What the shape is.
    pub kind: ShapeKind,
    /// The normalized path.
    pub path: BezPath,
    /// The fill colour.
    pub fill: Computed<WorkingColor>,
}

impl ResolvedShape {
    /// The Cherenkov shape filling `bounds`.
    #[must_use]
    pub fn resolve(&self, bounds: Rect) -> ShapeData {
        resolve_shape(self.kind, &self.path, bounds)
    }
}

waterui_core::raw_view!(ResolvedShape, waterui_core::layout::StretchAxis::Both);

/// A morphing shape as a backend receives it.
#[derive(Debug, Clone)]
pub struct ResolvedMorphShape {
    /// The shape at progress 0.
    pub from: ShapeKind,
    /// The shape at progress 1.
    pub to: ShapeKind,
    /// The fill colour.
    pub fill: Computed<WorkingColor>,
    /// The timing when `progress` is `None`.
    pub animation: MorphAnimation,
    /// An explicit progress signal, 0.0–1.0.
    pub progress: Option<Computed<f32>>,
}

impl waterui_core::NativeView for ResolvedMorphShape {
    fn stretch_axis(&self) -> waterui_core::layout::StretchAxis {
        waterui_core::layout::StretchAxis::Both
    }
}

// ============================================================================
// FilledShape - Shape as a View with backend-native fill rendering
// ============================================================================

/// A shape filled with a colour.
#[derive(Debug)]
pub struct FilledShape {
    kind: ShapeKind,
    path: BezPath,
    fill: Color,
}

impl FilledShape {
    /// Fills `shape` with `fill`, rendering it as a custom path.
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(shape: impl Shape, fill: impl Into<Color>) -> Self {
        Self {
            kind: ShapeKind::CustomPath,
            path: shape.path(),
            fill: fill.into(),
        }
    }

    #[allow(clippy::needless_pass_by_value)]
    fn with_kind(kind: ShapeKind, shape: impl Shape, fill: impl Into<Color>) -> Self {
        Self {
            kind,
            path: shape.path(),
            fill: fill.into(),
        }
    }

    /// The normalized path.
    #[must_use]
    pub const fn path(&self) -> &BezPath {
        &self.path
    }

    /// The fill colour.
    #[must_use]
    pub const fn fill(&self) -> &Color {
        &self.fill
    }

    /// What the shape is.
    #[must_use]
    pub const fn kind(&self) -> ShapeKind {
        self.kind
    }

    /// Morphs this shape into `target`.
    #[must_use]
    #[allow(clippy::needless_pass_by_value)]
    pub fn morph_to(self, target: impl ShapeExt) -> MorphShape {
        MorphShape::new(self.kind, target.shape_kind(), self.fill)
    }
}

/// Timing of a [`MorphShape`] without an explicit progress signal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MorphAnimation {
    /// The easing of one run; its duration is the run's length.
    pub curve: Curve,
    /// Whether the morph runs again after finishing.
    pub repeat: bool,
    /// Whether repeated runs alternate direction.
    pub autoreverse: bool,
}

impl Default for MorphAnimation {
    fn default() -> Self {
        Self {
            curve: Curve::ease_in_out(Duration::from_millis(900)),
            repeat: true,
            autoreverse: true,
        }
    }
}

impl MorphAnimation {
    /// A single run of `curve`.
    #[must_use]
    pub const fn once(curve: Curve) -> Self {
        Self {
            curve,
            repeat: false,
            autoreverse: false,
        }
    }
}

/// A shape morphing between two built-in shapes.
#[derive(Debug, Clone)]
pub struct MorphShape {
    from: ShapeKind,
    to: ShapeKind,
    fill: Color,
    animation: MorphAnimation,
    progress: Option<Computed<f32>>,
}

impl MorphShape {
    fn new(from: ShapeKind, to: ShapeKind, fill: Color) -> Self {
        Self {
            from,
            to,
            fill,
            animation: MorphAnimation::default(),
            progress: None,
        }
    }

    /// Sets the timing.
    #[must_use]
    pub const fn animation(mut self, animation: MorphAnimation) -> Self {
        self.animation = animation;
        self
    }

    /// Sets the run length.
    #[must_use]
    pub const fn duration(mut self, duration: Duration) -> Self {
        self.animation.curve.duration = duration;
        self
    }

    /// Sets the easing, keeping the run length.
    #[must_use]
    pub const fn curve(mut self, curve: Curve) -> Self {
        self.animation.curve = Curve {
            duration: self.animation.curve.duration,
            ..curve
        };
        self
    }

    /// Sets whether the morph repeats.
    #[must_use]
    pub const fn repeat(mut self, repeat: bool) -> Self {
        self.animation.repeat = repeat;
        self
    }

    /// Sets whether repeats alternate direction.
    #[must_use]
    pub const fn autoreverse(mut self, autoreverse: bool) -> Self {
        self.animation.autoreverse = autoreverse;
        self
    }

    /// Drives the morph from a progress signal (0.0–1.0) instead of the clock.
    #[must_use]
    pub fn progress(mut self, progress: impl IntoComputed<f32>) -> Self {
        self.progress = Some(progress.into_computed());
        self
    }
}

impl View for FilledShape {
    fn body(self, env: &Environment) -> impl View {
        ResolvedShape {
            kind: self.kind,
            path: self.path,
            fill: self.fill.resolve(env).computed(),
        }
    }

    fn stretch_axis(&self) -> waterui_core::layout::StretchAxis {
        waterui_core::layout::StretchAxis::Both
    }
}

impl View for MorphShape {
    fn body(self, env: &Environment) -> impl View {
        waterui_core::Native::new(ResolvedMorphShape {
            from: self.from,
            to: self.to,
            fill: self.fill.resolve(env).computed(),
            animation: self.animation,
            progress: self.progress,
        })
    }

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
    /// Morphing supports the built-in shapes:
    /// `Rectangle`, `Circle`, `Ellipse`, `RoundedRectangle`, `UnevenRoundedRectangle`, `Capsule`.
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
        for radius in [f32::NAN, -4.0] {
            match FixedRoundedRectangle::new(radius).shape_kind() {
                ShapeKind::FixedRoundedRect { corner_radius } => {
                    assert!((corner_radius - 0.0).abs() < 1e-6);
                }
                _ => panic!("unexpected kind"),
            }
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

    #[test]
    fn kinds_resolve_against_the_shorter_side() {
        let bounds = Rect::new(10.0, 20.0, 110.0, 60.0);
        match Circle.resolve(bounds) {
            ShapeData::Circle(circle) => {
                assert_eq!(circle.center, Point::new(60.0, 40.0));
                assert!((circle.radius - 20.0).abs() < 1e-9);
            }
            other => panic!("unexpected shape {other:?}"),
        }
        match RoundedRectangle::new(0.25).resolve(bounds) {
            ShapeData::RoundedRect(rounded) => {
                assert!((rounded.radii().top_left - 10.0).abs() < 1e-9);
            }
            other => panic!("unexpected shape {other:?}"),
        }
        match FixedRoundedRectangle::new(80.0).resolve(bounds) {
            ShapeData::RoundedRect(rounded) => {
                assert!((rounded.radii().top_left - 20.0).abs() < 1e-9);
            }
            other => panic!("unexpected shape {other:?}"),
        }
    }

    #[test]
    fn custom_paths_scale_per_axis() {
        let bounds = Rect::new(0.0, 0.0, 200.0, 100.0);
        let path = Path::new().move_to(0.0, 0.0).line_to(1.0, 1.0).close();
        match path.resolve(bounds) {
            ShapeData::Path { elements, .. } => {
                assert_eq!(elements[1], PathEl::LineTo(Point::new(200.0, 100.0)));
            }
            other => panic!("unexpected shape {other:?}"),
        }
    }

    #[test]
    fn capsule_path_stays_in_the_unit_square() {
        let path = Capsule.path();
        let bbox = path.bounding_box();
        assert!(bbox.x0 >= -1e-6 && bbox.y0 >= -1e-6);
        assert!(bbox.x1 <= 1.0 + 1e-6 && bbox.y1 <= 1.0 + 1e-6);
    }
}
