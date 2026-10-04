//! Prepared mesh paint.
//!
//! Each grid cell is a bilinear patch, in row-major
//! order, with corners 00, 10, 01, 11. Geometry and premultiplied linear-P3
//! colour use bilinear or smoothstep weights. Outside all patches is transparent.
//! Overlapping patches use the last patch; folded patches use the inverse
//! with greatest v, then greatest u. Collapsed, zero-Jacobian samples are
//! transparent. These ownership rules select one paint value, not a stack
//! of translucent draws. The enclosing shape supplies pixel coverage.

use cherenkov::MeshGradient;
use kurbo::{Affine, Point, Rect};
use std::sync::Arc;

/// Inverts a bilinear patch at `point`, selecting the last parameter-space
/// branch. Corner order is 00, 10, 01, 11.
#[must_use]
#[expect(
    clippy::many_single_char_names,
    reason = "bilinear polynomial coefficients and parameters"
)]
fn coordinates(corners: [Point; 4], point: Point) -> Option<(f64, f64)> {
    let [origin, right, bottom, opposite] = corners;
    let e = right - origin;
    let f = bottom - origin;
    let g = (opposite - bottom) - e;
    let q = point - origin;
    // cross(q - f*v, e + g*v) = 0.
    let a = -f.cross(g);
    let b = q.cross(g) - f.cross(e);
    let c = q.cross(e);
    let roots = if a == 0.0 {
        if b == 0.0 {
            return None;
        }
        [-c / b; 2]
    } else {
        let discriminant = b.mul_add(b, -4.0 * a * c);
        if discriminant < 0.0 {
            return None;
        }
        let t = -0.5 * (b + discriminant.sqrt().copysign(b));
        if t == 0.0 {
            [-b / (2.0 * a); 2]
        } else {
            [t / a, c / t]
        }
    };
    let mut result: Option<(f64, f64)> = None;
    for v in roots {
        if !(0.0..=1.0).contains(&v) {
            continue;
        }
        let direction = e + g * v;
        let residual = q - f * v;
        let u = if direction.x.abs() >= direction.y.abs() {
            residual.x / direction.x
        } else {
            residual.y / direction.y
        };
        if (0.0..=1.0).contains(&u)
            && direction.cross(f + g * u) != 0.0
            && result.is_none_or(|(old_u, old_v)| (v, u) > (old_v, old_u))
        {
            result = Some((u, v));
        }
    }
    result
}

/// A prepared bilinear patch with its conservative paint-space bounds.
#[derive(Debug)]
struct Patch {
    corners: [Point; 4],
    colors: [[f64; 4]; 4],
    bounds: Rect,
}

/// Shared prepared patches; cloning a paint does not duplicate its grid.
#[derive(Clone, Debug)]
pub struct Mesh {
    inverse: Affine,
    patches: Arc<[Patch]>,
    interpolation: cherenkov::MeshColorInterpolation,
}

impl Mesh {
    /// Prepare premultiplied colours and bounds once per lowered paint.
    pub fn new(mesh: &MeshGradient, inverse: Affine) -> Self {
        let stride = mesh.columns() as usize + 1;
        let mut patches = Vec::new();
        for row in 0..mesh.rows() as usize {
            for column in 0..mesh.columns() as usize {
                let index = row * stride + column;
                let indices = [index, index + 1, index + stride, index + stride + 1];
                let corners = indices.map(|i| mesh.points()[i]);
                let colors = indices.map(|i| {
                    let [red, green, blue, alpha] = mesh.colors()[i].components.map(f64::from);
                    [red * alpha, green * alpha, blue * alpha, alpha]
                });
                let bounds = corners.iter().fold(
                    Rect::new(corners[0].x, corners[0].y, corners[0].x, corners[0].y),
                    |bounds, &point| bounds.union_pt(point),
                );
                patches.push(Patch {
                    corners,
                    colors,
                    bounds,
                });
            }
        }
        Self {
            inverse,
            patches: patches.into(),
            interpolation: mesh.interpolation_mode(),
        }
    }

    /// Compose a device-to-content placement without copying the patch grid.
    pub fn transformed(&self, inverse: Affine) -> Self {
        Self {
            inverse: self.inverse * inverse,
            patches: Arc::clone(&self.patches),
            interpolation: self.interpolation,
        }
    }

    /// Heap bytes of the shared patch grid.
    pub fn heap_bytes(&self) -> u64 {
        (self.patches.len() * size_of::<Patch>()) as u64
    }

    /// Evaluate at the device pixel centre; coverage is supplied by the draw.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "paint rounds once to framebuffer precision"
    )]
    pub fn eval(&self, x: f32, y: f32) -> [f32; 4] {
        let point = self.inverse * Point::new(f64::from(x), f64::from(y));
        for patch in self.patches.iter().rev() {
            if point.x < patch.bounds.x0
                || point.x > patch.bounds.x1
                || point.y < patch.bounds.y0
                || point.y > patch.bounds.y1
            {
                continue;
            }
            if let Some((u, v)) = coordinates(patch.corners, point) {
                let (u, v) = match self.interpolation {
                    cherenkov::MeshColorInterpolation::Linear => (u, v),
                    cherenkov::MeshColorInterpolation::Smoothstep => {
                        (u * u * u.mul_add(-2.0, 3.0), v * v * v.mul_add(-2.0, 3.0))
                    }
                };
                let mut out = [0.0; 4];
                for (channel, value) in out.iter_mut().enumerate() {
                    let top = u.mul_add(
                        patch.colors[1][channel] - patch.colors[0][channel],
                        patch.colors[0][channel],
                    );
                    let bottom = u.mul_add(
                        patch.colors[3][channel] - patch.colors[2][channel],
                        patch.colors[2][channel],
                    );
                    *value = v.mul_add(bottom - top, top) as f32;
                }
                return out;
            }
        }
        [0.0; 4]
    }
}
