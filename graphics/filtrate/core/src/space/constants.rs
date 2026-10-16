//! The numeric definition of the working space and of sRGB: the
//! chromaticities both are built from, the matrices between them, the
//! working space's luma row and the sRGB transfer parameters.
//!
//! This file is the one source of these numbers. The crate re-exports them
//! from [`space`](super), and the crate's build script includes this file
//! to render the same values into the WGSL fragment every shader that
//! converts between the spaces is composed with, so the CPU and GPU paths
//! cannot drift apart.
//!
//! The matrices are derived here rather than written out: each colour
//! space's RGB → XYZ matrix follows from its primaries and white point
//! (SMPTE RP 177), computed in `f64` at compile time and rounded once to
//! `f32`.

/// A CIE 1931 xy chromaticity.
type Xy = [f64; 2];

/// A row-major 3×3 matrix.
type Mat3 = [[f64; 3]; 3];

/// The CIE standard illuminant D65 white point, shared by sRGB and
/// Display P3.
const D65: Xy = [0.3127, 0.3290];

/// The sRGB (BT.709) red, green and blue primaries.
const SRGB_PRIMARIES: [Xy; 3] = [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06]];

/// The Display P3 (DCI-P3 primaries) red, green and blue primaries.
const DISPLAY_P3_PRIMARIES: [Xy; 3] = [[0.680, 0.320], [0.265, 0.690], [0.150, 0.060]];

/// The XYZ of the chromaticity `xy` at luminance `Y = 1`.
const fn xyz([x, y]: Xy) -> [f64; 3] {
    [x / y, 1.0, (1.0 - x - y) / y]
}

/// `m · v`.
const fn apply(m: &Mat3, v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// `a · b`.
const fn product(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut out = [[0.0; 3]; 3];
    let mut row = 0;
    while row < 3 {
        let mut column = 0;
        while column < 3 {
            out[row][column] =
                a[row][0] * b[0][column] + a[row][1] * b[1][column] + a[row][2] * b[2][column];
            column += 1;
        }
        row += 1;
    }
    out
}

/// The 2×2 minor of `m` on rows `r0`, `r1` and columns `c0`, `c1`.
const fn minor(m: &Mat3, r0: usize, r1: usize, c0: usize, c1: usize) -> f64 {
    m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0]
}

/// The inverse of `m`, by its adjugate. The matrices inverted here are
/// colour-space bases, never singular.
const fn inverse(m: &Mat3) -> Mat3 {
    let adjugate = [
        [
            minor(m, 1, 2, 1, 2),
            -minor(m, 0, 2, 1, 2),
            minor(m, 0, 1, 1, 2),
        ],
        [
            -minor(m, 1, 2, 0, 2),
            minor(m, 0, 2, 0, 2),
            -minor(m, 0, 1, 0, 2),
        ],
        [
            minor(m, 1, 2, 0, 1),
            -minor(m, 0, 2, 0, 1),
            minor(m, 0, 1, 0, 1),
        ],
    ];
    let determinant =
        m[0][0] * adjugate[0][0] + m[0][1] * adjugate[1][0] + m[0][2] * adjugate[2][0];
    let mut out = [[0.0; 3]; 3];
    let mut row = 0;
    while row < 3 {
        let mut column = 0;
        while column < 3 {
            out[row][column] = adjugate[row][column] / determinant;
            column += 1;
        }
        row += 1;
    }
    out
}

/// The linear RGB → XYZ matrix of a colour space with these primaries and
/// white point: each primary's XYZ column scaled so that RGB `(1, 1, 1)`
/// lands on the white point at `Y = 1`.
const fn rgb_to_xyz(primaries: &[Xy; 3], white: Xy) -> Mat3 {
    let [r, g, b] = [xyz(primaries[0]), xyz(primaries[1]), xyz(primaries[2])];
    let basis = [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]];
    let scale = apply(&inverse(&basis), xyz(white));
    let mut out = basis;
    let mut row = 0;
    while row < 3 {
        let mut column = 0;
        while column < 3 {
            out[row][column] *= scale[column];
            column += 1;
        }
        row += 1;
    }
    out
}

/// `m`, each entry rounded to the nearest `f32`.
#[allow(
    clippy::cast_possible_truncation,
    reason = "rounding the f64 derivation to f32 once is the point"
)]
const fn narrow(m: &Mat3) -> [[f32; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    let mut row = 0;
    while row < 3 {
        let mut column = 0;
        while column < 3 {
            out[row][column] = m[row][column] as f32;
            column += 1;
        }
        row += 1;
    }
    out
}

/// Linear Display P3 → XYZ (D65).
const DISPLAY_P3_TO_XYZ: Mat3 = rgb_to_xyz(&DISPLAY_P3_PRIMARIES, D65);

/// Linear sRGB → XYZ (D65).
const SRGB_TO_XYZ: Mat3 = rgb_to_xyz(&SRGB_PRIMARIES, D65);

/// Linear Display P3 to linear sRGB (both D65), rows first.
pub const P3_TO_SRGB: [[f32; 3]; 3] = narrow(&product(&inverse(&SRGB_TO_XYZ), &DISPLAY_P3_TO_XYZ));

/// Linear sRGB to linear Display P3 (both D65), rows first.
pub const SRGB_TO_P3: [[f32; 3]; 3] = narrow(&product(&inverse(&DISPLAY_P3_TO_XYZ), &SRGB_TO_XYZ));

/// The luma coefficients of linear Display P3: the Y row of its RGB → XYZ
/// matrix.
pub const P3_LUMA: [f32; 3] = narrow(&DISPLAY_P3_TO_XYZ)[1];

/// The sRGB transfer's exponent `γ`, before rounding.
const GAMMA: f64 = 2.4;

/// The sRGB transfer's offset `a`, before rounding.
const OFFSET: f64 = 0.055;

/// The slope of the sRGB transfer's linear segment (IEC 61966-2-1).
pub const SRGB_SLOPE: f32 = 12.92;

/// The sRGB transfer's offset `a`: the power segment is
/// `(1 + a)·L^(1/γ) − a`.
#[allow(
    clippy::cast_possible_truncation,
    reason = "rounding the definition to f32 once is the point"
)]
pub const SRGB_OFFSET: f32 = OFFSET as f32;

/// `1 + a`, the scale of the transfer's power segment.
#[allow(
    clippy::cast_possible_truncation,
    reason = "rounding the definition to f32 once is the point"
)]
pub const SRGB_SCALE: f32 = (1.0 + OFFSET) as f32;

/// The sRGB transfer's exponent `γ`.
#[allow(
    clippy::cast_possible_truncation,
    reason = "rounding the definition to f32 once is the point"
)]
pub const SRGB_GAMMA: f32 = GAMMA as f32;

/// `1 / γ`, rounded once from the exact quotient.
#[allow(
    clippy::cast_possible_truncation,
    reason = "rounding the exact quotient to f32 once is the point"
)]
pub const SRGB_INVERSE_GAMMA: f32 = (1.0 / GAMMA) as f32;

/// The largest linear value on the transfer's linear segment.
pub const SRGB_LINEAR_BREAK: f32 = 0.003_130_8;

/// The largest encoded value on the transfer's linear segment.
pub const SRGB_ENCODED_BREAK: f32 = 0.040_45;

#[cfg(test)]
mod tests {
    use super::{P3_LUMA, P3_TO_SRGB, SRGB_TO_P3};

    fn times(a: &[[f32; 3]; 3], b: &[[f32; 3]; 3]) -> [[f64; 3]; 3] {
        std::array::from_fn(|row| {
            std::array::from_fn(|column| {
                (0..3)
                    .map(|k| f64::from(a[row][k]) * f64::from(b[k][column]))
                    .sum()
            })
        })
    }

    /// The derived matrices are each other's inverse and keep D65 white
    /// white, to the precision of their `f32` rounding.
    #[test]
    fn derived_matrices_are_inverse_and_keep_white() {
        for product in [
            times(&P3_TO_SRGB, &SRGB_TO_P3),
            times(&SRGB_TO_P3, &P3_TO_SRGB),
        ] {
            for (row, values) in product.iter().enumerate() {
                for (column, value) in values.iter().enumerate() {
                    let identity = if row == column { 1.0 } else { 0.0 };
                    assert!((value - identity).abs() < 1.0e-6, "{product:?}");
                }
            }
        }
        for matrix in [P3_TO_SRGB, SRGB_TO_P3] {
            for row in matrix {
                let white: f64 = row.iter().copied().map(f64::from).sum();
                assert!((white - 1.0).abs() < 1.0e-6, "{matrix:?}");
            }
        }
        let luminance: f64 = P3_LUMA.iter().copied().map(f64::from).sum();
        assert!((luminance - 1.0).abs() < 1.0e-6, "{P3_LUMA:?}");
    }
}
