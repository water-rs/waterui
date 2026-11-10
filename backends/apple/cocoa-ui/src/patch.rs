//! Bilinear-patch meshes as plain data, and their CPU rasterization.
//!
//! The one Rust statement of a mesh's patches and of the per-pixel inverse
//! that paints them: the Metal pass (`mesh_gradient.metal`, [`crate::mesh`])
//! uploads [`Patch`]es built here and solves the same inverse in MSL, and a
//! run Core Graphics draws on the CPU rasterizes with [`raster`], so a mesh
//! reads back no GPU pixels.
//!
//! A mesh is row-major bilinear quadrilateral patches. Each pixel inverts
//! the patches, last row-major patch first, at the pixel's point in the
//! mesh's own space; the first patch that contains the point owns it, and a
//! folded patch takes the inverse with the greater vertical coordinate,
//! then the greater horizontal one. Colours are premultiplied linear
//! Display P3, interpolated bilinearly (or smoothstep weighted). A point
//! outside every patch is transparent (`graphics/cherenkov/docs/mesh-paint.md`).
//!
//! # Edge coverage
//!
//! The engine's mesh paint leaves coverage to the shape that is filled. A
//! canvas realizes the shape as a mask, which antialiases the shape's own
//! boundary but not the boundary of the patches inside it, so the pixel's
//! colour here is the mean over [`SAMPLE_OFFSETS`] sub-pixel points: a pixel
//! the patch boundary crosses is covered in proportion. The pattern is eight
//! rooks, so every row and column of an 8x8 grid holds one sample.

/// One patch as the shader reads it: `repr(C)`, byte for byte `MeshPatch`
/// in `mesh_gradient.metal`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Patch {
    /// Corners 00 and 10, as x, y pairs.
    pub top: [f32; 4],
    /// Corners 01 and 11, as x, y pairs.
    pub bottom: [f32; 4],
    /// Premultiplied colours of corners 00, 10, 01, 11.
    pub colors: [[f32; 4]; 4],
}

/// The sub-pixel points a pixel's colour averages, as offsets from its
/// centre in pixels. The same table is `SAMPLE_OFFSETS` in the shader.
pub const SAMPLE_OFFSETS: [[f32; 2]; 8] = [
    [0.1875, -0.4375],
    [-0.1875, -0.3125],
    [0.4375, -0.1875],
    [-0.4375, -0.0625],
    [-0.0625, 0.0625],
    [0.3125, 0.1875],
    [-0.3125, 0.3125],
    [0.0625, 0.4375],
];

/// The patches of a `columns` by `rows` mesh over a
/// `(columns + 1) x (rows + 1)` vertex grid, row-major, with `points` in the
/// mesh's space and `colors` as straight linear Display P3 RGBA.
///
/// # Panics
///
/// When the grids do not match the dimensions, or the mesh has no patches.
#[must_use]
pub fn patches(
    columns: usize,
    rows: usize,
    points: &[[f64; 2]],
    colors: &[[f32; 4]],
) -> Vec<Patch> {
    let stride = columns + 1;
    let vertices = stride * (rows + 1);
    assert!(columns > 0 && rows > 0, "a mesh has at least one patch");
    assert_eq!(points.len(), vertices, "a mesh has a point per vertex");
    assert_eq!(colors.len(), vertices, "a mesh has a colour per vertex");
    let premultiplied = |index: usize| {
        let [red, green, blue, alpha] = colors[index];
        [red * alpha, green * alpha, blue * alpha, alpha]
    };
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the inverse is solved in f32, as the engine's GPU solve is"
    )]
    let corner = |index: usize| [points[index][0] as f32, points[index][1] as f32];
    let mut patches = Vec::with_capacity(columns * rows);
    for row in 0..rows {
        for column in 0..columns {
            let at = row * stride + column;
            let [a, b, c, d] = [at, at + 1, at + stride, at + stride + 1];
            patches.push(Patch {
                top: [corner(a)[0], corner(a)[1], corner(b)[0], corner(b)[1]],
                bottom: [corner(c)[0], corner(c)[1], corner(d)[0], corner(d)[1]],
                colors: [
                    premultiplied(a),
                    premultiplied(b),
                    premultiplied(c),
                    premultiplied(d),
                ],
            });
        }
    }
    patches
}

fn cross(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0].mul_add(b[1], -(a[1] * b[0]))
}

fn sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

const fn add_scaled(a: [f32; 2], b: [f32; 2], scale: f32) -> [f32; 2] {
    [scale.mul_add(b[0], a[0]), scale.mul_add(b[1], a[1])]
}

/// The coordinates `(u, v)` of `point` in the patch whose top and bottom
/// edges are `top` and `bottom`, or `None` when no inverse lies in the unit
/// square.
fn inverse(point: [f32; 2], top: [f32; 4], bottom: [f32; 4]) -> Option<[f32; 2]> {
    let origin = [top[0], top[1]];
    let horizontal = sub([top[2], top[3]], origin);
    let vertical = sub([bottom[0], bottom[1]], origin);
    let bend = sub(
        sub([bottom[2], bottom[3]], [bottom[0], bottom[1]]),
        horizontal,
    );
    let delta = sub(point, origin);
    let qa = -cross(vertical, bend);
    let qb = cross(delta, bend) - cross(vertical, horizontal);
    let qc = cross(delta, horizontal);
    let roots = if qa == 0.0 {
        if qb == 0.0 {
            return None;
        }
        [-qc / qb; 2]
    } else {
        let discriminant = qb.mul_add(qb, -(4.0 * qa * qc));
        if discriminant < 0.0 {
            return None;
        }
        let signed_root = discriminant.sqrt().copysign(qb);
        let numerator = -0.5 * (qb + signed_root);
        [
            numerator / qa,
            if numerator == 0.0 {
                numerator / qa
            } else {
                qc / numerator
            },
        ]
    };
    let mut answer: Option<[f32; 2]> = None;
    for v in roots {
        if !(0.0..=1.0).contains(&v) {
            continue;
        }
        let direction = add_scaled(horizontal, bend, v);
        let remainder = add_scaled(delta, vertical, -v);
        let u = if direction[0].abs() >= direction[1].abs() {
            if direction[0] == 0.0 {
                continue;
            }
            remainder[0] / direction[0]
        } else {
            remainder[1] / direction[1]
        };
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        if cross(direction, add_scaled(vertical, bend, u)) == 0.0 {
            continue;
        }
        if answer.is_none_or(|[old_u, old_v]| (v, u) > (old_v, old_u)) {
            answer = Some([u, v]);
        }
    }
    answer
}

/// The premultiplied colour the mesh gives `point`, in the mesh's space:
/// transparent outside every patch.
#[must_use]
pub fn color_at(patches: &[Patch], smooth: bool, point: [f32; 2]) -> [f32; 4] {
    for patch in patches.iter().rev() {
        let Some([mut u, mut v]) = inverse(point, patch.top, patch.bottom) else {
            continue;
        };
        if smooth {
            u = u * u * u.mul_add(-2.0, 3.0);
            v = v * v * v.mul_add(-2.0, 3.0);
        }
        let mix = |a: f32, b: f32, t: f32| t.mul_add(b - a, a);
        return core::array::from_fn(|channel| {
            let top = mix(patch.colors[0][channel], patch.colors[1][channel], u);
            let bottom = mix(patch.colors[2][channel], patch.colors[3][channel], u);
            mix(top, bottom, v)
        });
    }
    [0.0; 4]
}

/// A mesh's pixels: `width` by `height` premultiplied colours, row-major
/// from the top row.
///
/// `pixel_to_mesh` maps a position in pixels — the pixel `(x, y)`'s centre
/// is `(x + 0.5, y + 0.5)` — into the mesh's space, as `kurbo`'s six
/// coefficients. Each pixel is the mean of the mesh at its
/// [`SAMPLE_OFFSETS`].
#[must_use]
pub fn raster(
    patches: &[Patch],
    smooth: bool,
    width: usize,
    height: usize,
    pixel_to_mesh: [f32; 6],
) -> Vec<[f32; 4]> {
    let map = pixel_to_mesh;
    let mut pixels = Vec::with_capacity(width * height);
    #[expect(
        clippy::cast_precision_loss,
        reason = "pixel coordinates are far below f32's integer range"
    )]
    for row in 0..height {
        for column in 0..width {
            let centre = [column as f32 + 0.5, row as f32 + 0.5];
            let mut sum = [0.0_f32; 4];
            for offset in SAMPLE_OFFSETS {
                let (across, down) = (centre[0] + offset[0], centre[1] + offset[1]);
                let point = [
                    map[0].mul_add(across, map[2].mul_add(down, map[4])),
                    map[1].mul_add(across, map[3].mul_add(down, map[5])),
                ];
                let color = color_at(patches, smooth, point);
                for (sum, color) in sum.iter_mut().zip(color) {
                    *sum += color;
                }
            }
            #[expect(
                clippy::cast_precision_loss,
                reason = "the sample count is a small constant"
            )]
            let weight = 1.0 / SAMPLE_OFFSETS.len() as f32;
            pixels.push(sum.map(|sum| sum * weight));
        }
    }
    pixels
}
