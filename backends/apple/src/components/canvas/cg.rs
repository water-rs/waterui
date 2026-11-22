//! Recorded values as Core Graphics values: paths, colours, transforms.
//!
//! # Safety
//!
//! The `unsafe` calls build `CGPath`s and `CGColor`s from values this
//! module owns for the call: a null transform pointer is the documented
//! no-transform argument, and every pointer to a component array or
//! transform outlives its call.

use core::ptr;

use cocoa_ui::objc2_core_foundation::{CFRetained, CGAffineTransform, CGPoint, CGRect, CGSize};
use cocoa_ui::objc2_core_graphics::{
    CGColor, CGColorSpace, CGLineCap, CGLineJoin, CGMutablePath, CGPath,
    kCGColorSpaceExtendedLinearDisplayP3,
};
use kurbo::{Affine, Circle, Join, PathEl, Point, Rect, RoundedRect, RoundedRectRadii, Shape as _};
use kurbo::{Stroke, StrokeOpts};
use waterui_graphics::backend::draw::{
    ContinuousRect, FillRule, PATH_TOLERANCE, Paint, ShapeData, WorkingColor,
};

/// The path elements of `shape`.
#[must_use]
pub fn elements(shape: &ShapeData) -> Vec<PathEl> {
    match shape {
        ShapeData::Rect(rect) => rect.path_elements(PATH_TOLERANCE).collect(),
        ShapeData::RoundedRect(rect) => rect.path_elements(PATH_TOLERANCE).collect(),
        ShapeData::Continuous(rect) => continuous(rect).elements().to_vec(),
        ShapeData::Circle(circle) => circle.path_elements(PATH_TOLERANCE).collect(),
        ShapeData::Ellipse(ellipse) => ellipse.path_elements(PATH_TOLERANCE).collect(),
        ShapeData::Line(line) => line.path_elements(PATH_TOLERANCE).collect(),
        ShapeData::Path { elements, .. } => elements.to_vec(),
    }
}

fn continuous(rect: &ContinuousRect) -> kurbo::BezPath {
    rect.to_path(PATH_TOLERANCE)
}

/// How `shape` decides its interior.
#[must_use]
pub const fn fill_rule(shape: &ShapeData) -> FillRule {
    match shape {
        ShapeData::Path { rule, .. } => *rule,
        _ => FillRule::NonZero,
    }
}

/// `point` as a Core Graphics point.
#[must_use]
pub const fn point(point: Point) -> CGPoint {
    CGPoint::new(point.x, point.y)
}

/// `rect` as a Core Graphics rectangle.
#[must_use]
pub const fn rect(rect: Rect) -> CGRect {
    CGRect::new(
        CGPoint::new(rect.x0, rect.y0),
        CGSize::new(rect.x1 - rect.x0, rect.y1 - rect.y0),
    )
}

/// `affine` as a Core Graphics transform.
#[must_use]
pub const fn transform(affine: Affine) -> CGAffineTransform {
    let [a, b, c, d, tx, ty] = affine.as_coeffs();
    CGAffineTransform { a, b, c, d, tx, ty }
}

/// The path of `shape`.
#[must_use]
pub fn path(shape: &ShapeData) -> CFRetained<CGPath> {
    path_of(elements(shape))
}

/// The path of `elements`.
///
/// # Panics
///
/// If Core Graphics cannot copy a live path.
#[must_use]
pub fn path_of(elements: impl IntoIterator<Item = PathEl>) -> CFRetained<CGPath> {
    let path = CGMutablePath::new();
    append(&path, elements, Affine::IDENTITY);
    CGPath::new_copy(Some(&path)).expect("CGPathCreateCopy of a live path is non-null")
}

/// Appends `elements`, moved by `placement`, to `path`.
pub fn append(path: &CGMutablePath, elements: impl IntoIterator<Item = PathEl>, placement: Affine) {
    let at = |p: Point| {
        let p = placement * p;
        (p.x, p.y)
    };
    for element in elements {
        // SAFETY: `path` is a live mutable path; a null transform is the
        // documented no-transform argument.
        unsafe {
            match element {
                PathEl::MoveTo(p) => {
                    let (x, y) = at(p);
                    CGMutablePath::move_to_point(Some(path), ptr::null(), x, y);
                }
                PathEl::LineTo(p) => {
                    let (x, y) = at(p);
                    CGMutablePath::add_line_to_point(Some(path), ptr::null(), x, y);
                }
                PathEl::QuadTo(c, p) => {
                    let ((cx, cy), (x, y)) = (at(c), at(p));
                    CGMutablePath::add_quad_curve_to_point(Some(path), ptr::null(), cx, cy, x, y);
                }
                PathEl::CurveTo(c1, c2, p) => {
                    let ((c1x, c1y), (c2x, c2y), (x, y)) = (at(c1), at(c2), at(p));
                    CGMutablePath::add_curve_to_point(
                        Some(path),
                        ptr::null(),
                        c1x,
                        c1y,
                        c2x,
                        c2y,
                        x,
                        y,
                    );
                }
                PathEl::ClosePath => CGMutablePath::close_subpath(Some(path)),
            }
        }
    }
}

/// The outline of `elements` stroked with `stroke`, which a nonzero fill
/// covers exactly as the stroke does: each end gets its own cap and dashes
/// are cut, which a single Core Animation or Core Graphics line cap cannot
/// express when the ends differ.
#[must_use]
pub fn stroke_outline(
    elements: impl IntoIterator<Item = PathEl>,
    stroke: &Stroke,
) -> kurbo::BezPath {
    kurbo::stroke(elements, stroke, &StrokeOpts::default(), PATH_TOLERANCE)
}

/// How far a stroke reaches beyond its path.
#[must_use]
pub fn reach(stroke: Option<&Stroke>) -> f64 {
    stroke.map_or(0.0, |stroke| {
        let miter = if stroke.join == Join::Miter {
            stroke.miter_limit.max(1.0)
        } else {
            1.0
        };
        (stroke.width / 2.0).mul_add(miter, 1.0)
    })
}

/// The box `path` covers, or `None` when it covers nothing.
#[must_use]
pub fn bounds(path: &CGPath) -> Option<Rect> {
    if CGPath::is_empty(Some(path)) {
        return None;
    }
    let bounds = CGPath::bounding_box(Some(path));
    Some(Rect::new(
        bounds.origin.x,
        bounds.origin.y,
        bounds.origin.x + bounds.size.width,
        bounds.origin.y + bounds.size.height,
    ))
}

/// The region `shape` covers, grown by `spread`, or shrunk when it is
/// negative: the region dilated (eroded) by a disc of radius `|spread|`,
/// the morphology the engine's shadow spread applies.
///
/// Rectangles, rounded rectangles and circles offset exactly by their own
/// parameters. Any other shape is its region united with — or less — the
/// band `2|spread|` wide around its outline, with round joins and caps,
/// through Core Graphics' path operations: every point within `|spread|`
/// of the region is in the region or within `|spread|` of its outline.
///
/// A path operation Core Graphics refuses is logged and leaves the shape
/// unspread.
#[must_use]
pub fn spread(shape: &ShapeData, spread: f64) -> CFRetained<CGPath> {
    if spread == 0.0 {
        return path(shape);
    }
    let analytic = match shape {
        ShapeData::Rect(rect) => Some(ShapeData::Rect(rect.inflate(spread, spread))),
        ShapeData::RoundedRect(rounded) => {
            let radii = rounded.radii();
            Some(ShapeData::RoundedRect(RoundedRect::from_rect(
                rounded.rect().inflate(spread, spread),
                RoundedRectRadii::new(
                    (radii.top_left + spread).max(0.0),
                    (radii.top_right + spread).max(0.0),
                    (radii.bottom_right + spread).max(0.0),
                    (radii.bottom_left + spread).max(0.0),
                ),
            )))
        }
        ShapeData::Circle(circle) => Some(ShapeData::Circle(Circle::new(
            circle.center,
            (circle.radius + spread).max(0.0),
        ))),
        ShapeData::Ellipse(_)
        | ShapeData::Continuous(_)
        | ShapeData::Line(_)
        | ShapeData::Path { .. } => None,
    };
    if let Some(analytic) = analytic {
        return path(&analytic);
    }
    let source = path(shape);
    let spread_of = |source: &CGPath| {
        let region =
            CGPath::new_copy_by_normalizing(Some(source), fill_rule(shape) == FillRule::EvenOdd)?;
        let band = |outline: &CGPath| {
            // SAFETY: a null transform is the documented no-transform argument.
            unsafe {
                CGPath::new_copy_by_stroking_path(
                    Some(outline),
                    ptr::null(),
                    2.0 * spread.abs(),
                    CGLineCap::Round,
                    CGLineJoin::Round,
                    1.0,
                )
            }
        };
        if spread > 0.0 {
            // Growing, the band follows the shape's own outline: a line or any
            // other region-less part grows too, and the band of an edge inside
            // the region lies inside the grown region anyway.
            CGPath::new_copy_by_unioning_path(Some(&region), Some(&*band(source)?), false)
        } else {
            // Shrinking, only the region's boundary eats into it.
            CGPath::new_copy_by_subtracting_path(Some(&region), Some(&*band(&region)?), false)
        }
    };
    spread_of(&source).unwrap_or_else(|| {
        tracing::error!(
            "Core Graphics refused a path operation for a spread of {spread}; \
             the shape is not spread"
        );
        source
    })
}

/// Whether a colour needs more than SDR.
#[must_use]
pub fn color_extended(color: WorkingColor) -> bool {
    color.components[..3]
        .iter()
        .any(|component| !(0.0..=1.0).contains(component))
}

/// Whether any colour `paint` shows needs more than SDR.
#[must_use]
pub fn paint_extended(paint: &Paint) -> bool {
    match paint {
        Paint::Solid(color) => color_extended(*color),
        Paint::Linear(g) => g.stops.iter().any(|s| color_extended(s.color)),
        Paint::Radial(g) => g.stops.iter().any(|s| color_extended(s.color)),
        Paint::Sweep(g) => g.stops.iter().any(|s| color_extended(s.color)),
        Paint::Mesh(mesh) => mesh.colors().iter().copied().any(color_extended),
        Paint::Transformed(t) => paint_extended(&t.paint),
        Paint::Image(_) | Paint::Shader(_) => false,
    }
}

/// The colour space the working colours live in: extended linear Display
/// P3, so values above SDR white and outside the gamut survive.
///
/// # Panics
///
/// If Core Graphics lacks the space.
#[must_use]
pub fn working_space() -> CFRetained<CGColorSpace> {
    // SAFETY: the static is a `CFString` constant Core Graphics exports.
    CGColorSpace::with_name(Some(unsafe { kCGColorSpaceExtendedLinearDisplayP3 }))
        .expect("Core Graphics provides extended linear Display P3")
}

/// `color` as a `CGColor` in the working space.
#[must_use]
pub fn color(color: WorkingColor) -> CFRetained<CGColor> {
    let [red, green, blue, alpha] = color.components;
    cocoa_ui::color::cg_extended_linear_display_p3(
        f64::from(red),
        f64::from(green),
        f64::from(blue),
        f64::from(alpha),
    )
}
