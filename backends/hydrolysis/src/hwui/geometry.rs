//! Transforms and bounds for the HWUI target.

use waterui_graphics::draw::kurbo::{Affine, Point, Rect};

/// An Android `Matrix` in `getValues` order.
pub type Matrix = [f32; 9];

/// Below this, a transform coefficient or a bounds edge counts as exact.
const EPSILON: f64 = 1e-6;

/// `affine` as an Android matrix.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "the wire carries f32, as android.graphics.Matrix stores"
)]
#[expect(
    clippy::many_single_char_names,
    reason = "a–f are the affine coefficients as kurbo and android.graphics.Matrix name them"
)]
pub const fn affine_matrix(affine: Affine) -> Matrix {
    let [a, b, c, d, e, f] = affine.as_coeffs();
    [
        a as f32, c as f32, e as f32, b as f32, d as f32, f as f32, 0., 0., 1.,
    ]
}

/// A plane homography (rows of `(x, y, 1) → (X, Y, W)`) as an Android
/// matrix, perspective row included.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "the wire carries f32, as android.graphics.Matrix stores"
)]
pub fn homography_matrix(rows: [[f64; 3]; 3]) -> Matrix {
    let mut out = [0.0f32; 9];
    for (index, value) in rows.iter().flatten().enumerate() {
        out[index] = *value as f32;
    }
    out
}

/// Whether `affine` is the identity, within [`EPSILON`].
#[must_use]
pub fn is_identity(affine: Affine) -> bool {
    affine
        .as_coeffs()
        .iter()
        .zip(Affine::IDENTITY.as_coeffs())
        .all(|(a, b)| (a - b).abs() <= EPSILON)
}

/// An affine map without skew: `translate · rotate · scale`, the
/// composition a `RenderNode`'s properties express.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decomposed {
    /// Translation.
    pub translation: (f64, f64),
    /// Scale factors; `y` is negative for a reflection.
    pub scale: (f64, f64),
    /// Rotation about Z, in degrees.
    pub rotation: f64,
}

/// Splits `affine` into node properties, or `None` when it skews (its
/// columns are not orthogonal) or collapses the X axis.
#[must_use]
#[expect(
    clippy::many_single_char_names,
    reason = "a–f are the affine coefficients as kurbo and android.graphics.Matrix name them"
)]
pub fn decompose(affine: Affine) -> Option<Decomposed> {
    let [a, b, c, d, e, f] = affine.as_coeffs();
    let sx = a.hypot(b);
    if sx <= EPSILON {
        return None;
    }
    let angle = b.atan2(a);
    let sy = a.mul_add(d, -(b * c)) / sx;
    let (sin, cos) = angle.sin_cos();
    let tolerance = EPSILON * sx.max(sy.abs()).max(1.0);
    if f64::mul_add(sy, sin, c).abs() > tolerance || f64::mul_add(sy, -cos, d).abs() > tolerance {
        return None;
    }
    Some(Decomposed {
        translation: (e, f),
        scale: (sx, sy),
        rotation: angle.to_degrees(),
    })
}

/// `a ∪ b`, where `None` is empty.
#[must_use]
pub const fn union(a: Option<Rect>, b: Option<Rect>) -> Option<Rect> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.union(b)),
        (a, None) => a,
        (None, b) => b,
    }
}

/// `a ∩ b`, where `None` is empty.
#[must_use]
pub fn intersect(a: Option<Rect>, b: Rect) -> Option<Rect> {
    let out = a?.intersect(b);
    (out.width() > 0.0 && out.height() > 0.0).then_some(out)
}

/// The bounding box of the part of `rect` that a plane homography maps in
/// front of its eye: the projected quad is clipped to `W > ε` first, so the
/// box is finite. `None` when no part of `rect` is in front.
#[must_use]
pub fn project_rect(rows: Rows, rect: Rect) -> Option<Rect> {
    let corners = [
        Point::new(rect.x0, rect.y0),
        Point::new(rect.x1, rect.y0),
        Point::new(rect.x1, rect.y1),
        Point::new(rect.x0, rect.y1),
    ]
    .map(|corner| homogeneous(rows, corner));
    let mut out: Option<Rect> = None;
    let mut add = |[x, y, w]: [f64; 3]| {
        let point = Point::new(x / w, y / w);
        out = Some(out.map_or_else(|| Rect::from_points(point, point), |r| r.union_pt(point)));
    };
    for (index, &from) in corners.iter().enumerate() {
        let to = corners[(index + 1) % corners.len()];
        if from[2] > EPSILON {
            add(from);
        }
        if (from[2] > EPSILON) != (to[2] > EPSILON) {
            let t = (EPSILON - from[2]) / (to[2] - from[2]);
            add([
                t.mul_add(to[0] - from[0], from[0]),
                t.mul_add(to[1] - from[1], from[1]),
                EPSILON,
            ]);
        }
    }
    out
}

/// `point` under `rows`, before the divide by `W`.
fn homogeneous(rows: Rows, point: Point) -> [f64; 3] {
    rows.map(|row| row[0].mul_add(point.x, row[1].mul_add(point.y, row[2])))
}

/// A plane homography, row-major.
pub type Rows = [[f64; 3]; 3];

/// Skia's points per inch: a `RenderNode`'s camera distance is in inches.
const POINTS_PER_INCH: f64 = 72.0;

/// The camera distance, in pixels, a tilt about one axis is expressed
/// with: the image does not determine it, and `RenderNode`'s default is
/// 8 inches.
const DEFAULT_CAMERA: f64 = 8.0 * POINTS_PER_INCH;

/// HWUI's `MathUtils::isZero` tolerance: when both tilts are below it, the
/// node ignores its camera.
const NON_ZERO_EPSILON: f64 = 0.001;

/// How far, in pixels, a node's image of a probe point may stray from the
/// homography it expresses.
const MATCH_TOLERANCE: f64 = 1.0 / 256.0;

/// A plane homography expressed as `RenderNode` properties: scale about the
/// pivot, then the node camera's tilt and rotation about it, then
/// translation. `RenderProperties::updateMatrix` builds the same matrix.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tilted {
    /// The homography this expresses, layer space to parent space.
    pub rows: Rows,
    /// Translation of the pivot, in parent space.
    pub translation: (f64, f64),
    /// The pivot, in layer space; the camera sits in front of it.
    pub pivot: (f64, f64),
    /// Scale factors about the pivot.
    pub scale: (f64, f64),
    /// Rotation about Z, in degrees.
    pub rotation: f64,
    /// Tilt about X, in degrees.
    pub rotation_x: f64,
    /// Tilt about Y, in degrees.
    pub rotation_y: f64,
    /// The camera's distance from the plane, in inches.
    pub camera_distance: f64,
}

/// Expresses `rows` as `RenderNode` camera properties, or `None` when no
/// node camera reproduces it: no perspective, a skew, depth, or a pose the
/// node's f32 properties cannot match within [`MATCH_TOLERANCE`].
///
/// A node image is `T(c) · C · S · T(-p)`, `C` a rotation `R` under a
/// perspective of distance `d`, so `H` decomposes once the pivot's image `c`
/// and `d` are chosen: the columns of `T(-c) · H · T(p)` are then
/// `sx·(R₀₀, R₁₀, -d·g)` and `sy·(R₀₁, R₁₁, -d·h)`, which must be orthogonal.
/// The image of the layer origin is tried first, as a node pivots there;
/// otherwise the `c` that best satisfies orthogonality. The result is
/// verified against `rows` through a model of `updateMatrix`.
#[must_use]
#[expect(
    clippy::many_single_char_names,
    reason = "a–h are the homography coefficients of the derivation above"
)]
pub fn tilt(rows: Rows) -> Option<Tilted> {
    let magnitude = rows.iter().flatten().fold(0.0_f64, |m, v| m.max(v.abs()));
    if !magnitude.is_finite() || magnitude <= EPSILON {
        return None;
    }
    let h = rows.map(|row| row.map(|v| v / magnitude));
    let [[h00, h01, _], [h10, h11, _], [g, gy, h22]] = h;
    let flat = |v: f64| v.abs() <= EPSILON * EPSILON;
    if flat(g) && flat(gy) {
        return None;
    }
    let skew = |cx: f64, cy: f64| {
        cx.mul_add(-g, h00).mul_add(
            cx.mul_add(-gy, h01),
            cy.mul_add(-g, h10) * cy.mul_add(-gy, h11),
        )
    };
    let origin = (h22 > EPSILON).then(|| (h[0][2] / h22, h[1][2] / h22));
    let (c, d) = if flat(g) || flat(gy) {
        let normal = (g.mul_add(h01, gy * h00), g.mul_add(h11, gy * h10));
        let length = normal.0.hypot(normal.1);
        if length <= EPSILON * EPSILON {
            return None;
        }
        let target = h00.mul_add(h01, h10 * h11);
        let (ox, oy) = origin.unwrap_or((0.0, 0.0));
        let offset = (target - normal.0.mul_add(ox, normal.1 * oy)) / (length * length);
        (
            (normal.0.mul_add(offset, ox), normal.1.mul_add(offset, oy)),
            DEFAULT_CAMERA,
        )
    } else {
        let depth = |(cx, cy): (f64, f64)| {
            let squared = -skew(cx, cy) / (g * gy);
            (squared.is_finite() && squared > 0.0).then(|| squared.sqrt())
        };
        let best = (
            h00.mul_add(gy, h01 * g) / (2.0 * g * gy),
            h10.mul_add(gy, h11 * g) / (2.0 * g * gy),
        );
        match origin.and_then(|o| depth(o).map(|d| (o, d))) {
            Some(found) => found,
            None => (best, depth(best)?),
        }
    };
    let pivot = preimage(h, c)?;
    let w = homogeneous(h, Point::new(pivot.0, pivot.1))[2];
    if w <= EPSILON {
        return None;
    }
    let n = h.map(|row| row.map(|v| v / w));
    let (gn, hn) = (n[2][0], n[2][1]);
    let u = [
        c.0.mul_add(-gn, n[0][0]),
        c.1.mul_add(-gn, n[1][0]),
        -d * gn,
    ];
    let v = [
        c.0.mul_add(-hn, n[0][1]),
        c.1.mul_add(-hn, n[1][1]),
        -d * hn,
    ];
    let (sx, sy) = (norm(u), norm(v));
    if sx <= EPSILON || sy <= EPSILON {
        return None;
    }
    let r0 = u.map(|x| x / sx);
    let r1 = v.map(|x| x / sy);
    let r2 = [
        r0[1].mul_add(r1[2], -(r0[2] * r1[1])),
        r0[2].mul_add(r1[0], -(r0[0] * r1[2])),
        r0[0].mul_add(r1[1], -(r0[1] * r1[0])),
    ];
    let tilted = Tilted {
        rows,
        translation: (c.0 - pivot.0, c.1 - pivot.1),
        pivot,
        scale: (sx, sy),
        rotation: (-r1[0]).atan2(r0[0]).to_degrees(),
        rotation_x: (-r2[1]).atan2(r2[2]).to_degrees(),
        rotation_y: r2[0].clamp(-1.0, 1.0).asin().to_degrees(),
        camera_distance: d / POINTS_PER_INCH,
    };
    matches(&node_rows(&tilted), &rows, pivot).then_some(tilted)
}

/// The layer point `rows` maps to `image`, or `None` when it is singular.
#[expect(
    clippy::many_single_char_names,
    reason = "a–i are the homography's coefficients row by row, as the adjugate formula names them"
)]
fn preimage(rows: Rows, image: (f64, f64)) -> Option<(f64, f64)> {
    let [[a, b, c], [d, e, f], [g, h, i]] = rows;
    let adjugate = [
        [
            e.mul_add(i, -(f * h)),
            c.mul_add(h, -(b * i)),
            b.mul_add(f, -(c * e)),
        ],
        [
            f.mul_add(g, -(d * i)),
            a.mul_add(i, -(c * g)),
            c.mul_add(d, -(a * f)),
        ],
        [
            d.mul_add(h, -(e * g)),
            b.mul_add(g, -(a * h)),
            a.mul_add(e, -(b * d)),
        ],
    ];
    let [x, y, w] = homogeneous(adjugate, Point::new(image.0, image.1));
    let point = (x / w, y / w);
    (w.abs() > EPSILON * EPSILON && point.0.is_finite() && point.1.is_finite()).then_some(point)
}

fn norm(v: [f64; 3]) -> f64 {
    v[0].hypot(v[1]).hypot(v[2])
}

/// Whether `node` and `rows` map probe points around `pivot` to the same
/// place, and the same side of the eye.
fn matches(node: &Rows, rows: &Rows, pivot: (f64, f64)) -> bool {
    [16.0, 256.0].iter().all(|&reach| {
        [
            (-1.0, -1.0),
            (1.0, -1.0),
            (1.0, 1.0),
            (-1.0, 1.0),
            (0.0, 0.0),
        ]
        .iter()
        .all(|&(dx, dy): &(f64, f64)| {
            let probe = Point::new(dx.mul_add(reach, pivot.0), dy.mul_add(reach, pivot.1));
            let [ax, ay, aw] = homogeneous(*node, probe);
            let [bx, by, bw] = homogeneous(*rows, probe);
            match (aw > EPSILON, bw > EPSILON) {
                (true, true) => (ax / aw - bx / bw).hypot(ay / aw - by / bw) <= MATCH_TOLERANCE,
                (false, false) => true,
                _ => false,
            }
        })
    })
}

/// The homography a `RenderNode` with these properties applies, from layer
/// space: a port of `RenderProperties::updateMatrix`, `Sk3DView` and
/// `SkCamera3D::patchToMatrix`, on the properties rounded to the f32 the
/// node stores.
fn node_rows(tilted: &Tilted) -> Rows {
    let stored = |v: f64| f64::from(narrow(v));
    let (px, py) = (stored(tilted.pivot.0), stored(tilted.pivot.1));
    let (tx, ty) = (stored(tilted.translation.0), stored(tilted.translation.1));
    let (sx, sy) = (stored(tilted.scale.0), stored(tilted.scale.1));
    let rotation = stored(tilted.rotation);
    let (rx, ry) = (stored(tilted.rotation_x), stored(tilted.rotation_y));
    let scale = multiply(
        translate(px, py),
        multiply(
            [[sx, 0.0, 0.0], [0.0, sy, 0.0], [0.0, 0.0, 1.0]],
            translate(-px, -py),
        ),
    );
    if rx.abs() < NON_ZERO_EPSILON && ry.abs() < NON_ZERO_EPSILON {
        let (sin, cos) = rotation.to_radians().sin_cos();
        let turn = [[cos, -sin, 0.0], [sin, cos, 0.0], [0.0, 0.0, 1.0]];
        return multiply(
            translate(tx + px, ty + py),
            multiply(turn, multiply(translate(-px, -py), scale)),
        );
    }
    let view = multiply(
        axis_rotation([1.0, 0.0, 0.0], rx),
        multiply(
            axis_rotation([0.0, -1.0, 0.0], ry),
            axis_rotation([0.0, 0.0, 1.0], -rotation),
        ),
    );
    let u = [view[0][0], view[1][0], view[2][0]];
    let v = [-view[0][1], -view[1][1], -view[2][1]];
    // `RenderNode.setCameraDistance` negates; `Sk3DView` scales inches to points.
    let location = -stored(tilted.camera_distance) * POINTS_PER_INCH;
    let dot = -location;
    let camera = [
        [-location * u[0] / dot, -location * v[0] / dot, 0.0],
        [location * u[1] / dot, location * v[1] / dot, 0.0],
        [u[2] / dot, v[2] / dot, 1.0],
    ];
    multiply(
        translate(tx + px, ty + py),
        multiply(camera, multiply(translate(-px, -py), scale)),
    )
}

/// `Skia`'s `SkM44::setRotateUnitSinCos` about `axis`, by `degrees`.
#[expect(
    clippy::many_single_char_names,
    reason = "x, y, z, s, c and t are the names SkM44::setRotateUnitSinCos uses"
)]
fn axis_rotation([x, y, z]: [f64; 3], degrees: f64) -> Rows {
    let (s, c) = degrees.to_radians().sin_cos();
    let t = 1.0 - c;
    [
        [
            (t * x).mul_add(x, c),
            (t * x).mul_add(y, -s * z),
            (t * x).mul_add(z, s * y),
        ],
        [
            (t * x).mul_add(y, s * z),
            (t * y).mul_add(y, c),
            (t * y).mul_add(z, -s * x),
        ],
        [
            (t * x).mul_add(z, -s * y),
            (t * y).mul_add(z, s * x),
            (t * z).mul_add(z, c),
        ],
    ]
}

const fn translate(x: f64, y: f64) -> Rows {
    [[1.0, 0.0, x], [0.0, 1.0, y], [0.0, 0.0, 1.0]]
}

fn multiply(a: Rows, b: Rows) -> Rows {
    std::array::from_fn(|i| {
        std::array::from_fn(|j| {
            a[i][0].mul_add(b[0][j], a[i][1].mul_add(b[1][j], a[i][2] * b[2][j]))
        })
    })
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "a RenderNode stores its properties as f32"
)]
const fn narrow(value: f64) -> f32 {
    value as f32
}

/// Whether `value` is an integer, within [`EPSILON`].
#[must_use]
pub fn is_integral(value: f64) -> bool {
    (value - value.round()).abs() <= EPSILON
}

/// `value` rounded down to an `i32`, saturating.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "float-to-int `as` saturates, which is the intent at the i32 edge"
)]
pub const fn floor_i32(value: f64) -> i32 {
    value.floor() as i32
}

/// `value` rounded up to an `i32`, saturating.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "float-to-int `as` saturates, which is the intent at the i32 edge"
)]
pub const fn ceil_i32(value: f64) -> i32 {
    value.ceil() as i32
}

#[cfg(test)]
mod tests {
    use waterui_graphics::draw::kurbo::{Affine, Point, Rect, Vec2};

    use waterui_graphics::draw::Projective;

    use super::{Rows, Tilted, decompose, homogeneous, node_rows, project_rect, tilt};

    #[test]
    fn translate_rotate_scale_decomposes_and_skew_does_not() {
        let affine = Affine::translate(Vec2::new(3.0, 4.0))
            * Affine::rotate(std::f64::consts::FRAC_PI_2)
            * Affine::scale_non_uniform(2.0, -0.5);
        let parts = decompose(affine).unwrap();
        assert!((parts.rotation - 90.0).abs() < 1e-9);
        assert!((parts.scale.0 - 2.0).abs() < 1e-9 && (parts.scale.1 + 0.5).abs() < 1e-9);
        assert_eq!(parts.translation, (3.0, 4.0));
        assert!(decompose(Affine::skew(0.3, 0.0)).is_none());
        assert!(decompose(Affine::scale(0.0)).is_none());
    }

    #[test]
    fn a_projection_partly_behind_the_eye_is_clipped_to_finite_bounds() {
        let flat = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let rect = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert_eq!(project_rect(flat, rect), Some(rect));
        let behind = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [-0.2, 0.0, 1.0]];
        let clipped = project_rect(behind, rect).expect("the left half is in front");
        assert!(clipped.is_finite(), "{clipped:?}");
        assert!((clipped.x0 - 0.0).abs() < 1e-9 && (clipped.y0 - 0.0).abs() < 1e-9);
        assert!(clipped.x1 > 1e5 && clipped.y1 > 1e5, "{clipped:?}");
        let gone = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]];
        assert!(project_rect(gone, rect).is_none());
    }

    /// `translate(c) · perspective(d) · Rz · Ry · Rx · scale · translate(−p)`,
    /// composed as Cherenkov composes a pose and reduced to the plane.
    fn pose(
        centre: (f64, f64),
        distance: f64,
        [about_x, about_y, about_z]: [f64; 3],
        scale: (f64, f64),
        pivot: (f64, f64),
    ) -> Rows {
        let (sx, cx) = about_x.sin_cos();
        let (sy, cy) = about_y.sin_cos();
        let (sz, cz) = about_z.sin_cos();
        let rows = |m: [[f64; 4]; 4]| Projective::from_rows(m).expect("the factor is valid");
        let factors = [
            rows([
                [1.0, 0.0, 0.0, centre.0],
                [0.0, 1.0, 0.0, centre.1],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ]),
            Projective::perspective(distance).expect("the distance is valid"),
            rows([
                [cz, -sz, 0.0, 0.0],
                [sz, cz, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ]),
            rows([
                [cy, 0.0, sy, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [-sy, 0.0, cy, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ]),
            rows([
                [1.0, 0.0, 0.0, 0.0],
                [0.0, cx, -sx, 0.0],
                [0.0, sx, cx, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ]),
            rows([
                [scale.0, 0.0, 0.0, 0.0],
                [0.0, scale.1, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ]),
            rows([
                [1.0, 0.0, 0.0, -pivot.0],
                [0.0, 1.0, 0.0, -pivot.1],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ]),
        ];
        factors
            .into_iter()
            .try_fold(Projective::IDENTITY, Projective::checked_mul)
            .expect("the pose composes")
            .plane_homography()
    }

    fn assert_reproduces(rows: Rows, tilted: &Tilted) {
        let node = node_rows(tilted);
        for x in [0.0, 37.0, 120.0, 300.0] {
            for y in [0.0, 21.0, 80.0, 200.0] {
                let point = Point::new(x, y);
                let [ax, ay, aw] = homogeneous(node, point);
                let [bx, by, bw] = homogeneous(rows, point);
                assert!(aw > 0.0 && bw > 0.0, "{point:?} is in front of both");
                let miss = (ax / aw - bx / bw).hypot(ay / aw - by / bw);
                assert!(miss < 1e-2, "{point:?} misses by {miss}: {tilted:?}");
            }
        }
    }

    #[test]
    fn a_tilt_about_one_axis_becomes_node_camera_properties() {
        for angles in [
            [0.4, 0.0, 0.0],
            [0.0, -0.6, 0.0],
            [-1.1, 0.0, 0.0],
            [0.0, 0.9, 0.0],
        ] {
            let rows = pose((140.0, 90.0), 800.0, angles, (1.0, 1.0), (60.0, 40.0));
            let tilted = tilt(rows).unwrap_or_else(|| panic!("{angles:?} is a node camera"));
            assert!(
                tilted.rotation_x != 0.0 || tilted.rotation_y != 0.0,
                "{tilted:?}"
            );
            assert_reproduces(rows, &tilted);
        }
    }

    #[test]
    fn a_compound_tilt_with_rotation_and_scale_becomes_node_camera_properties() {
        for (angles, scale, distance) in [
            ([0.3, 0.5, 0.0], (1.0, 1.0), 600.0),
            ([0.2, -0.4, 0.7], (1.5, 0.75), 1200.0),
            ([-0.8, 0.3, -2.0], (0.5, 2.0), 350.0),
        ] {
            let rows = pose((200.0, 150.0), distance, angles, scale, (50.0, 25.0));
            let tilted = tilt(rows).unwrap_or_else(|| panic!("{angles:?} is a node camera"));
            assert_reproduces(rows, &tilted);
        }
    }

    #[test]
    fn a_pose_without_perspective_has_no_node_camera() {
        let rows = [[1.0, 0.2, 5.0], [0.0, 0.8, 3.0], [0.0, 0.0, 1.0]];
        assert!(tilt(rows).is_none());
        let orthographic = [[1.0, 0.0, 0.0], [0.0, 0.5, 0.0], [0.0, 0.0, 1.0]];
        assert!(tilt(orthographic).is_none());
    }
}
