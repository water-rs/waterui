//! Projective (perspective) layer transforms (#84).
//!
//! A [`Projective`] is a 4×4 homogeneous transform on column vectors. A
//! layer that carries one is a flattening boundary: its content, clip,
//! filter and children render into a layer-local image with the ordinary
//! affine rasterizers, and the completed image is projected when it is
//! composed into its parent. Recorded content stays affine.

use kurbo::{Affine, Vec2};

/// A homogeneous transform using column vectors.
///
/// `rows[row][column]` is row-major storage: a point `(x, y, z, 1)` maps to
/// `rows · (x, y, z, 1)ᵀ`. Positive Z is toward the viewer, and x and y
/// follow the layer's downward-y coordinate system. Positive homogeneous
/// scaling preserves the front half-space (`w > 0`); `M` and `−M` are
/// different transforms, because they select opposite half-spaces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projective {
    rows: [[f64; 4]; 4],
}

/// Why a [`Projective`] could not be constructed or composed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProjectiveError {
    /// A coefficient is NaN or infinite.
    #[error("the transform has a non-finite coefficient")]
    NonFinite,
    /// The matrix has no inverse.
    #[error("the transform is not invertible")]
    NonInvertible,
    /// [`Projective::perspective`] needs a finite distance above zero.
    #[error("the perspective distance must be finite and positive")]
    InvalidPerspectiveDistance,
}

nami_core::impl_constant!(Projective);

impl Projective {
    /// The identity transform.
    pub const IDENTITY: Self = Self {
        rows: [
            [1., 0., 0., 0.],
            [0., 1., 0., 0.],
            [0., 0., 1., 0.],
            [0., 0., 0., 1.],
        ],
    };

    /// A transform from row-major coefficients.
    ///
    /// # Errors
    /// [`ProjectiveError::NonFinite`] when a coefficient is NaN or infinite,
    /// [`ProjectiveError::NonInvertible`] when the matrix is singular.
    pub fn from_rows(rows: [[f64; 4]; 4]) -> Result<Self, ProjectiveError> {
        let value = Self { rows };
        value.validate()?;
        Ok(value)
    }

    /// A perspective camera at `distance` in front of the `z = 0` plane:
    /// `w = 1 − z / distance` before any other transform, so content
    /// moved toward the viewer grows and content moved away shrinks.
    ///
    /// # Errors
    /// [`ProjectiveError::InvalidPerspectiveDistance`] unless `distance`
    /// is finite and above zero.
    pub fn perspective(distance: f64) -> Result<Self, ProjectiveError> {
        if !distance.is_finite() || distance <= 0.0 {
            return Err(ProjectiveError::InvalidPerspectiveDistance);
        }
        let mut rows = Self::IDENTITY.rows;
        rows[3][2] = -1.0 / distance;
        Self::from_rows(rows)
    }

    /// The row-major coefficients.
    #[must_use]
    pub const fn as_rows(&self) -> &[[f64; 4]; 4] {
        &self.rows
    }

    /// Returns `self * rhs`: `rhs` is applied first.
    ///
    /// # Errors
    /// [`ProjectiveError::NonFinite`] when the product overflows,
    /// [`ProjectiveError::NonInvertible`] when it is singular.
    pub fn checked_mul(self, rhs: Self) -> Result<Self, ProjectiveError> {
        Self::from_rows(mul(&self.rows, &rhs.rows))
    }

    fn validate(&self) -> Result<(), ProjectiveError> {
        if !self.rows.iter().flatten().all(|v| v.is_finite()) {
            return Err(ProjectiveError::NonFinite);
        }
        if invert(&self.rows).is_none() {
            return Err(ProjectiveError::NonInvertible);
        }
        Ok(())
    }

    /// The 3×3 homography this transform induces on the `z = 0` plane:
    /// rows and columns 0, 1 and 3, mapping `(x, y, 1)` to `(X, Y, W)`.
    /// A valid pose whose plane is seen edge-on induces a singular
    /// homography; that is an empty contribution, not an error.
    #[must_use]
    pub const fn plane_homography(&self) -> [[f64; 3]; 3] {
        let r = &self.rows;
        [
            [r[0][0], r[0][1], r[0][3]],
            [r[1][0], r[1][1], r[1][3]],
            [r[3][0], r[3][1], r[3][3]],
        ]
    }
}

impl TryFrom<Affine> for Projective {
    type Error = ProjectiveError;

    /// Embeds a 2D affine map in the `z = 0` plane, leaving `z` unchanged.
    fn try_from(value: Affine) -> Result<Self, Self::Error> {
        Self::from_rows(embed(value))
    }
}

/// The components a projective layer's pose is composed from, in the
/// order documented in `docs/api.md`.
#[derive(Clone, Copy, Debug)]
pub struct Pose {
    /// The layer's affine base (`transform`).
    pub base: Affine,
    /// The component translation.
    pub translation: Vec2,
    /// The component pivot.
    pub pivot: Vec2,
    /// The component rotation about Z, radians.
    pub rotation: f64,
    /// The component skew angles, radians.
    pub skew: Vec2,
    /// The component scale factors.
    pub scale: Vec2,
    /// The projection base.
    pub projection: Projective,
    /// Rotation about X (`tilt.x`) and Y (`tilt.y`), radians.
    pub tilt: Vec2,
    /// Translation along Z.
    pub depth: f64,
}

impl Pose {
    /// `embed(B) · T(t + p) · P · T(0, 0, z) · Rz(r) · Ry(v) · Rx(u) ·
    /// embed(K · S) · T(−p)`, validated.
    pub fn matrix(&self) -> Result<Projective, ProjectiveError> {
        let skew_scale = Affine::new([1., self.skew.y.tan(), self.skew.x.tan(), 1., 0., 0.])
            * Affine::scale_non_uniform(self.scale.x, self.scale.y);
        let lead = self.translation + self.pivot;
        let factors = [
            embed(self.base),
            translate3(lead.x, lead.y, 0.0),
            self.projection.rows,
            translate3(0.0, 0.0, self.depth),
            rotate_z(self.rotation),
            rotate_y(self.tilt.y),
            rotate_x(self.tilt.x),
            embed(skew_scale),
            translate3(-self.pivot.x, -self.pivot.y, 0.0),
        ];
        let rows = factors
            .iter()
            .skip(1)
            .fold(factors[0], |acc, factor| mul(&acc, factor));
        Projective::from_rows(rows)
    }
}

type M4 = [[f64; 4]; 4];

fn mul(a: &M4, b: &M4) -> M4 {
    std::array::from_fn(|i| {
        std::array::from_fn(|j| {
            a[i][3].mul_add(
                b[3][j],
                a[i][2].mul_add(b[2][j], a[i][0].mul_add(b[0][j], a[i][1] * b[1][j])),
            )
        })
    })
}

/// `Affine [a b c d e f]` embedded in the `z = 0` plane.
#[expect(
    clippy::many_single_char_names,
    reason = "a..f are the conventional affine coefficient names"
)]
const fn embed(t: Affine) -> M4 {
    let [a, b, c, d, e, f] = t.as_coeffs();
    [
        [a, c, 0., e],
        [b, d, 0., f],
        [0., 0., 1., 0.],
        [0., 0., 0., 1.],
    ]
}

const fn translate3(x: f64, y: f64, z: f64) -> M4 {
    [
        [1., 0., 0., x],
        [0., 1., 0., y],
        [0., 0., 1., z],
        [0., 0., 0., 1.],
    ]
}

/// Rotation about Z with the same sense as `Affine::rotate`.
fn rotate_z(angle: f64) -> M4 {
    let (s, c) = angle.sin_cos();
    [
        [c, -s, 0., 0.],
        [s, c, 0., 0.],
        [0., 0., 1., 0.],
        [0., 0., 0., 1.],
    ]
}

fn rotate_y(angle: f64) -> M4 {
    let (s, c) = angle.sin_cos();
    [
        [c, 0., s, 0.],
        [0., 1., 0., 0.],
        [-s, 0., c, 0.],
        [0., 0., 0., 1.],
    ]
}

fn rotate_x(angle: f64) -> M4 {
    let (s, c) = angle.sin_cos();
    [
        [1., 0., 0., 0.],
        [0., c, -s, 0.],
        [0., s, c, 0.],
        [0., 0., 0., 1.],
    ]
}

/// The inverse by Gauss-Jordan elimination with partial pivoting; `None`
/// when a pivot is exactly zero or the result is not finite.
fn invert(m: &M4) -> Option<M4> {
    let mut a = *m;
    let mut inv = Projective::IDENTITY.rows;
    for col in 0..4 {
        let pivot = (col..4).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[pivot][col] == 0.0 {
            return None;
        }
        a.swap(col, pivot);
        inv.swap(col, pivot);
        let p = a[col][col];
        for j in 0..4 {
            a[col][j] /= p;
            inv[col][j] /= p;
        }
        for i in 0..4 {
            if i != col {
                let f = a[i][col];
                if f != 0.0 {
                    for j in 0..4 {
                        a[i][j] = (-f).mul_add(a[col][j], a[i][j]);
                        inv[i][j] = (-f).mul_add(inv[col][j], inv[i][j]);
                    }
                }
            }
        }
    }
    inv.iter().flatten().all(|v| v.is_finite()).then_some(inv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, FRAC_PI_6, PI};

    fn apply(m: &Projective, p: [f64; 3]) -> [f64; 4] {
        let r = m.as_rows();
        std::array::from_fn(|i| {
            r[i][2].mul_add(p[2], r[i][1].mul_add(p[1], r[i][0].mul_add(p[0], r[i][3])))
        })
    }

    fn pose(projection: Projective) -> Pose {
        Pose {
            base: Affine::IDENTITY,
            translation: Vec2::ZERO,
            pivot: Vec2::ZERO,
            rotation: 0.0,
            skew: Vec2::ZERO,
            scale: Vec2::new(1.0, 1.0),
            projection,
            tilt: Vec2::ZERO,
            depth: 0.0,
        }
    }

    #[test]
    fn perspective_divides_by_one_minus_z_over_distance() {
        let camera = Projective::perspective(800.0).unwrap();
        let [x, y, _, w] = apply(&camera, [10.0, 20.0, 400.0]);
        let bits = |v: f64| v.to_bits();
        assert_eq!(bits(w), bits(0.5));
        assert_eq!([bits(x / w), bits(y / w)], [bits(20.0), bits(40.0)]);
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                Projective::perspective(bad),
                Err(ProjectiveError::InvalidPerspectiveDistance)
            );
        }
    }

    #[test]
    fn construction_rejects_non_finite_and_singular_matrices() {
        let mut rows = Projective::IDENTITY.rows;
        rows[1][2] = f64::NAN;
        assert_eq!(Projective::from_rows(rows), Err(ProjectiveError::NonFinite));
        let mut rows = Projective::IDENTITY.rows;
        rows[2] = [0.0; 4];
        assert_eq!(
            Projective::from_rows(rows),
            Err(ProjectiveError::NonInvertible)
        );
        assert_eq!(
            Projective::try_from(Affine::scale(0.0)),
            Err(ProjectiveError::NonInvertible)
        );
        let big = Projective::from_rows([
            [1e200, 0., 0., 0.],
            [0., 1., 0., 0.],
            [0., 0., 1., 0.],
            [0., 0., 0., 1.],
        ])
        .unwrap();
        assert_eq!(big.checked_mul(big), Err(ProjectiveError::NonFinite));
    }

    #[test]
    fn checked_mul_applies_the_right_operand_first() {
        let shift = Projective::try_from(Affine::translate((5.0, 0.0))).unwrap();
        let double = Projective::try_from(Affine::scale(2.0)).unwrap();
        let [x, ..] = apply(&shift.checked_mul(double).unwrap(), [1.0, 0.0, 0.0]);
        assert_eq!(x.to_bits(), 7.0_f64.to_bits());
    }

    #[test]
    fn identity_components_reduce_to_the_affine_composition() {
        let mut p = pose(Projective::IDENTITY);
        p.base = Affine::translate((13.0, 17.0)) * Affine::rotate(0.3);
        p.translation = Vec2::new(2.0, 3.0);
        p.pivot = Vec2::new(4.0, 5.0);
        p.rotation = 0.7;
        p.skew = Vec2::new(0.2, -0.1);
        p.scale = Vec2::new(2.0, 3.0);
        let affine = p.base
            * Affine::translate(p.translation + p.pivot)
            * Affine::rotate(p.rotation)
            * Affine::new([1., p.skew.y.tan(), p.skew.x.tan(), 1., 0., 0.])
            * Affine::scale_non_uniform(p.scale.x, p.scale.y)
            * Affine::translate(-p.pivot);
        let m = p.matrix().unwrap();
        assert_eq!(
            m.as_rows()[3].map(f64::to_bits),
            [0., 0., 0., 1.].map(f64::to_bits)
        );
        let expected = embed(affine);
        for (row, want) in m.as_rows().iter().zip(expected) {
            for (a, b) in row.iter().zip(want) {
                assert!((a - b).abs() < 1e-12, "{a} != {b}");
            }
        }
    }

    #[test]
    fn tilt_about_a_pivot_keeps_the_pivot_axis_fixed() {
        let mut p = pose(Projective::perspective(800.0).unwrap());
        p.pivot = Vec2::new(160.0, 100.0);
        p.tilt = Vec2::new(0.0, FRAC_PI_6);
        let m = p.matrix().unwrap();
        // Points on the vertical line through the pivot stay put under a
        // rotation about Y.
        for y in [0.0, 100.0, 200.0] {
            let [x, yy, _, w] = apply(&m, [160.0, y, 0.0]);
            assert!((x / w - 160.0).abs() < 1e-9);
            assert!((yy / w - y).abs() < 1e-9);
        }
        // The right edge moves away from the viewer and shrinks toward
        // the pivot.
        let [x, _, _, w] = apply(&m, [320.0, 100.0, 0.0]);
        assert!(w > 1.0 && x / w < 320.0);
    }

    #[test]
    fn an_edge_on_pose_is_valid_and_its_plane_homography_is_singular() {
        let mut p = pose(Projective::perspective(800.0).unwrap());
        p.tilt = Vec2::new(0.0, FRAC_PI_2);
        let m = p.matrix().unwrap();
        let h = m.plane_homography();
        let det = crate::lowering::projective::Homography(h).determinant();
        assert!(det.abs() < 1e-12, "det {det}");
    }

    #[test]
    fn a_half_turn_shows_the_back_side_mirrored() {
        let mut p = pose(Projective::perspective(800.0).unwrap());
        p.pivot = Vec2::new(100.0, 0.0);
        p.tilt = Vec2::new(0.0, PI);
        let m = p.matrix().unwrap();
        let [x, _, _, w] = apply(&m, [150.0, 0.0, 0.0]);
        assert!(w > 0.0);
        assert!((x / w - 50.0).abs() < 1e-9);
    }
}
