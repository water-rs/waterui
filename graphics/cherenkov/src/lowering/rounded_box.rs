//! The centred rounded box both backends evaluate a primitive shape as.
//!
//! Every primitive [`ShapeData`] with area is a box centred at the origin
//! with per-corner radii and a Lamé corner exponent, placed by a local
//! affine. The GPU packs the box into its instance data and evaluates its
//! signed distance in WGSL; the CPU evaluates the same distance in Rust.
//! Both read the box from [`box_form`], so the two engines cannot drift
//! apart in how a shape maps to it. The oracle keeps its own independent
//! `f64` mapping.

use kurbo::{Affine, RoundedRectRadii};

use crate::ShapeData;

/// A rounded box centred at the origin, in `f32` like the engines'
/// distance fields.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoundedBox {
    /// Half extents of the box.
    pub half: [f32; 2],
    /// Corner radius aspect: the y radius over the x radius.
    pub aspect: f32,
    /// The corner curve's Lamé exponent: `2.0` for a circular or
    /// elliptical corner, larger for a continuous corner.
    pub exponent: f32,
    /// Corner radii along x: top-left, top-right, bottom-right,
    /// bottom-left.
    pub radii: [f32; 4],
}

impl RoundedBox {
    /// A sharp box of half extents `half`.
    #[must_use]
    pub const fn rect(half: [f32; 2]) -> Self {
        Self {
            half,
            aspect: 1.0,
            exponent: 2.0,
            radii: [0.0; 4],
        }
    }
}

/// A shape's analytic form as a centred rounded box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoxForm {
    /// The shape is `shape` placed by the local affine `extra` (the
    /// centre translation, plus the rotation of an ellipse).
    Box {
        /// Local placement of the centred box.
        extra: Affine,
        /// The centred box.
        shape: RoundedBox,
    },
    /// The shape has no area: a line, or a circle or ellipse with a zero
    /// radius.
    Empty,
    /// A path, which has no analytic box.
    Path,
}

/// `shape` as a centred rounded box.
///
/// Corner radii clamp to the smaller half extent, a continuous corner's
/// smoothing maps to its Lamé exponent `2 + 2·smoothing`, and an ellipse
/// is a box whose corners are its quarter arcs.
#[must_use]
pub fn box_form(shape: &ShapeData) -> BoxForm {
    let (extra, shape) = match shape {
        ShapeData::Rect(r) => (
            Affine::translate(r.center().to_vec2()),
            RoundedBox::rect([f32_f64(r.width() / 2.0), f32_f64(r.height() / 2.0)]),
        ),
        ShapeData::RoundedRect(rr) => {
            let r = rr.rect();
            let half = [f32_f64(r.width() / 2.0), f32_f64(r.height() / 2.0)];
            (
                Affine::translate(r.center().to_vec2()),
                RoundedBox {
                    half,
                    aspect: 1.0,
                    exponent: 2.0,
                    radii: clamped_radii(rr.radii(), half),
                },
            )
        }
        ShapeData::Continuous(c) => {
            let r = c.rect;
            let half = [f32_f64(r.width() / 2.0), f32_f64(r.height() / 2.0)];
            (
                Affine::translate(r.center().to_vec2()),
                RoundedBox {
                    half,
                    aspect: 1.0,
                    exponent: f32_f64(c.smoothing).clamp(0.0, 1.0).mul_add(2.0, 2.0),
                    radii: clamped_radii(c.radii, half),
                },
            )
        }
        ShapeData::Circle(c) => {
            if c.radius <= 0.0 {
                return BoxForm::Empty;
            }
            let r = f32_f64(c.radius);
            (
                Affine::translate(c.center.to_vec2()),
                RoundedBox {
                    half: [r, r],
                    aspect: 1.0,
                    exponent: 2.0,
                    radii: [r; 4],
                },
            )
        }
        ShapeData::Ellipse(e) => {
            let radii = e.radii();
            let (a, b) = (radii.x, radii.y);
            if a <= 0.0 || b <= 0.0 {
                return BoxForm::Empty;
            }
            (
                Affine::translate(e.center().to_vec2()) * Affine::rotate(e.rotation()),
                RoundedBox {
                    half: [f32_f64(a), f32_f64(b)],
                    aspect: f32_f64(b / a),
                    exponent: 2.0,
                    radii: [f32_f64(a); 4],
                },
            )
        }
        ShapeData::Line(_) => return BoxForm::Empty,
        ShapeData::Path { .. } => return BoxForm::Path,
    };
    BoxForm::Box { extra, shape }
}

/// Corner radii clamped to `[0, min(half)]`.
fn clamped_radii(radii: RoundedRectRadii, half: [f32; 2]) -> [f32; 4] {
    let limit = f64::from(half[0].min(half[1]));
    [
        radii.top_left,
        radii.top_right,
        radii.bottom_right,
        radii.bottom_left,
    ]
    .map(|r| f32_f64(r.clamp(0.0, limit)))
}

/// `f64` to `f32`: the box is the engines' `f32` distance-field input.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the engines evaluate distance fields in f32"
)]
const fn f32_f64(v: f64) -> f32 {
    v as f32
}
