//! Resolving a [`ShapeKind`] against the size the shape is drawn at.
//!
//! A shape's [`PathCommand`](waterui_shape::PathCommand) list is normalized per
//! axis, so resolving it against a non-square rect stretches every corner by the
//! aspect ratio: a 200x50 surface with a rounded-rectangle clip gets flat
//! elliptical corners sweeping most of its edge instead of round ones. The kind
//! carries what the commands cannot — a corner radius as a fraction of the
//! *shorter* side — so this module resolves the kind and the commands are only
//! consulted for [`ShapeKind::CustomPath`].
//!
//! Both the fill ([`crate::components::graphics::shape`]) and the clip
//! ([`crate::components::graphics::clip_shape_widget`]) resolve through here, so
//! a clip always matches the fill of the same shape.
//!
//! This module is platform independent on purpose: it is plain geometry, and it
//! compiles and is tested on every host even though the rest of the backend is
//! Linux only.

use waterui_shape::ShapeKind;

/// One corner's radii in points, which an ellipse makes unequal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Corner {
    /// Radius along the x axis.
    pub horizontal: f64,
    /// Radius along the y axis.
    pub vertical: f64,
}

impl Corner {
    /// A square corner.
    pub const SQUARE: Self = Self {
        horizontal: 0.0,
        vertical: 0.0,
    };

    /// A corner whose two radii are equal, which is what every kind but
    /// [`ShapeKind::Ellipse`] produces.
    #[must_use]
    pub const fn circular(radius: f64) -> Self {
        Self {
            horizontal: radius,
            vertical: radius,
        }
    }
}

/// An axis-aligned rounded rectangle in widget-local points.
///
/// Every non-custom [`ShapeKind`] is one of these: a plain rect has square
/// corners, and a circle or an ellipse is a rect whose corners are half its own
/// width and height, which leaves no straight edge between them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoundedRect {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
    /// Corner radii, clockwise from the top left.
    pub corners: [Corner; 4],
}

impl RoundedRect {
    /// Whether every corner is square, so a plain rect describes this shape.
    #[must_use]
    pub fn is_rectangular(&self) -> bool {
        self.corners
            .iter()
            .all(|corner| corner.horizontal <= 0.0 || corner.vertical <= 0.0)
    }
}

/// A [`ShapeKind`] resolved against a concrete size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShapeGeometry {
    /// A rounded rectangle, which covers every kind but a custom path.
    Rounded(RoundedRect),
    /// Only the unit-space path commands describe this shape.
    CustomPath,
}

/// Resolves `kind` against a `width` x `height` box.
///
/// Normalized radii are a fraction of the shorter side, so they come out equal
/// on both axes and a corner stays circular however wide the box is.
#[must_use]
pub fn resolve(kind: ShapeKind, width: f64, height: f64) -> ShapeGeometry {
    let width = width.max(0.0);
    let height = height.max(0.0);
    let shorter = width.min(height);
    let limit = shorter / 2.0;
    let circular = |radius: f32| Corner::circular((f64::from(radius) * shorter).clamp(0.0, limit));
    let full = |corners: [Corner; 4]| {
        ShapeGeometry::Rounded(RoundedRect {
            x: 0.0,
            y: 0.0,
            width,
            height,
            corners,
        })
    };

    match kind {
        ShapeKind::Rect => full([Corner::SQUARE; 4]),
        // A circle is inscribed in the box: centred, its diameter the shorter
        // side, so it stays a circle rather than becoming an ellipse.
        ShapeKind::Circle => ShapeGeometry::Rounded(RoundedRect {
            x: (width - shorter) / 2.0,
            y: (height - shorter) / 2.0,
            width: shorter,
            height: shorter,
            corners: [Corner::circular(limit); 4],
        }),
        ShapeKind::Ellipse => full(
            [Corner {
                horizontal: width / 2.0,
                vertical: height / 2.0,
            }; 4],
        ),
        ShapeKind::RoundedRect { corner_radius } => full([circular(corner_radius); 4]),
        ShapeKind::UnevenRoundedRect {
            top_left,
            top_right,
            bottom_right,
            bottom_left,
        } => full([
            circular(top_left),
            circular(top_right),
            circular(bottom_right),
            circular(bottom_left),
        ]),
        ShapeKind::Capsule => full([Corner::circular(limit); 4]),
        ShapeKind::CustomPath => ShapeGeometry::CustomPath,
    }
}

#[cfg(test)]
mod tests {
    use super::{Corner, RoundedRect, ShapeGeometry, resolve};
    use waterui_shape::ShapeKind;

    fn rounded(kind: ShapeKind, width: f64, height: f64) -> RoundedRect {
        match resolve(kind, width, height) {
            ShapeGeometry::Rounded(rect) => rect,
            ShapeGeometry::CustomPath => panic!("expected a rounded rectangle, got a custom path"),
        }
    }

    /// Normalized radii are `f32`, so `12 / 50` comes back as 11.9999997 once
    /// it has been resolved in `f64`. Compare to the point rather than the bit.
    #[track_caller]
    fn assert_corners(actual: [Corner; 4], expected: [Corner; 4]) {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-4;
        assert!(
            actual
                .iter()
                .zip(expected.iter())
                .all(|(a, b)| close(a.horizontal, b.horizontal) && close(a.vertical, b.vertical)),
            "corners {actual:?} are not approximately {expected:?}"
        );
    }

    /// The defect this module exists for: a wide, short surface used to get
    /// corners stretched by its aspect ratio, so a 200x50 snackbar's 12pt
    /// corners swept 48pt horizontally. A normalized radius resolves against the
    /// shorter side and lands on the same number in both axes.
    #[test]
    fn rounded_rect_corners_are_circular_on_a_wide_rect() {
        let rect = rounded(
            ShapeKind::RoundedRect {
                corner_radius: 12.0 / 50.0,
            },
            200.0,
            50.0,
        );
        assert_corners(rect.corners, [Corner::circular(12.0); 4]);
        assert!(!rect.is_rectangular());
    }

    /// The same shape rotated: the radius follows the shorter side, whichever
    /// axis that is.
    #[test]
    fn rounded_rect_corners_are_circular_on_a_tall_rect() {
        let rect = rounded(
            ShapeKind::RoundedRect {
                corner_radius: 12.0 / 50.0,
            },
            50.0,
            200.0,
        );
        assert_corners(rect.corners, [Corner::circular(12.0); 4]);
    }

    #[test]
    fn rounded_rect_radius_saturates_at_half_the_shorter_side() {
        let rect = rounded(ShapeKind::RoundedRect { corner_radius: 0.9 }, 200.0, 50.0);
        assert_corners(rect.corners, [Corner::circular(25.0); 4]);
    }

    #[test]
    fn uneven_rounded_rect_keeps_each_corner_circular() {
        let rect = rounded(
            ShapeKind::UnevenRoundedRect {
                top_left: 0.2,
                top_right: 0.1,
                bottom_right: 0.0,
                bottom_left: 0.4,
            },
            200.0,
            50.0,
        );
        assert_corners(
            rect.corners,
            [
                Corner::circular(10.0),
                Corner::circular(5.0),
                Corner::SQUARE,
                Corner::circular(20.0),
            ],
        );
    }

    #[test]
    fn capsule_caps_are_half_the_shorter_side() {
        let rect = rounded(ShapeKind::Capsule, 200.0, 50.0);
        assert_corners(rect.corners, [Corner::circular(25.0); 4]);
        assert_eq!(
            (rect.x, rect.y, rect.width, rect.height),
            (0.0, 0.0, 200.0, 50.0)
        );
    }

    #[test]
    fn circle_is_inscribed_and_centred() {
        let rect = rounded(ShapeKind::Circle, 200.0, 50.0);
        assert_eq!(
            (rect.x, rect.y, rect.width, rect.height),
            (75.0, 0.0, 50.0, 50.0)
        );
        assert_corners(rect.corners, [Corner::circular(25.0); 4]);
    }

    /// An ellipse is the one kind whose corners are deliberately unequal: it
    /// fills the box, so each "corner" is half the box in that axis.
    #[test]
    fn ellipse_fills_the_bounds() {
        let rect = rounded(ShapeKind::Ellipse, 200.0, 50.0);
        assert_eq!(
            (rect.x, rect.y, rect.width, rect.height),
            (0.0, 0.0, 200.0, 50.0)
        );
        assert_corners(
            rect.corners,
            [Corner {
                horizontal: 100.0,
                vertical: 25.0,
            }; 4],
        );
    }

    #[test]
    fn rect_has_square_corners() {
        let rect = rounded(ShapeKind::Rect, 200.0, 50.0);
        assert_eq!(rect.corners, [Corner::SQUARE; 4]);
        assert!(rect.is_rectangular());
    }

    #[test]
    fn custom_path_has_no_resolved_geometry() {
        assert_eq!(
            resolve(ShapeKind::CustomPath, 200.0, 50.0),
            ShapeGeometry::CustomPath
        );
    }

    #[test]
    fn a_degenerate_box_produces_no_radius() {
        let rect = rounded(ShapeKind::Capsule, 200.0, 0.0);
        assert_eq!(rect.corners, [Corner::SQUARE; 4]);
        assert!(rect.is_rectangular());
    }
}
