//! Bilinear mesh reference. Solve the inverse in f64 along the horizontal
//! coordinate, independently of the GPU's vertical-coordinate polynomial.

use cherenkov_scene::MeshGradient;
use kurbo::{Point, Vec2};

fn inverse(point: Point, corners: [Point; 4]) -> Option<(f64, f64)> {
    let horizontal = corners[1] - corners[0];
    let vertical = corners[2] - corners[0];
    let bend = corners[3] - corners[2] - horizontal;
    let delta = point - corners[0];
    let quadratic = -horizontal.cross(bend);
    let linear = delta.cross(bend) - horizontal.cross(vertical);
    let constant = delta.cross(vertical);
    let roots = if quadratic == 0.0 {
        if linear == 0.0 {
            return None;
        }
        [-constant / linear; 2]
    } else {
        let discriminant = linear.mul_add(linear, -4.0 * quadratic * constant);
        if discriminant < 0.0 {
            return None;
        }
        let numerator = -0.5 * (linear + discriminant.sqrt().copysign(linear));
        let first = numerator / quadratic;
        [
            first,
            if numerator == 0.0 {
                first
            } else {
                constant / numerator
            },
        ]
    };
    let mut selected: Option<(f64, f64)> = None;
    for u in roots {
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let direction = vertical + u * bend;
        let remainder = delta - u * horizontal;
        let v = divide(remainder, direction);
        if !(0.0..=1.0).contains(&v) || (horizontal + v * bend).cross(direction) == 0.0 {
            continue;
        }
        if selected.is_none_or(|(old_u, old_v)| (v, u) > (old_v, old_u)) {
            selected = Some((u, v));
        }
    }
    selected
}

fn divide(numerator: Vec2, denominator: Vec2) -> f64 {
    if denominator.x.abs() >= denominator.y.abs() {
        numerator.x / denominator.x
    } else {
        numerator.y / denominator.y
    }
}

pub fn eval(mesh: &MeshGradient, point: Point) -> [f64; 4] {
    let columns = mesh.columns() as usize;
    let stride = columns + 1;
    for row in (0..mesh.rows() as usize).rev() {
        for column in (0..columns).rev() {
            let base = row * stride + column;
            let indices = [base, base + 1, base + stride, base + stride + 1];
            if let Some((u, v)) = inverse(point, indices.map(|index| mesh.points()[index])) {
                let (u, v) = match mesh.interpolation_mode() {
                    cherenkov_scene::MeshColorInterpolation::Linear => (u, v),
                    cherenkov_scene::MeshColorInterpolation::Smoothstep => (
                        u.powi(2) * (-2.0_f64).mul_add(u, 3.0),
                        v.powi(2) * (-2.0_f64).mul_add(v, 3.0),
                    ),
                };
                let colors = indices.map(|index| crate::color::to_working(&mesh.colors()[index]));
                let weights = [(1.0 - u) * (1.0 - v), u * (1.0 - v), (1.0 - u) * v, u * v];
                return std::array::from_fn(|channel| {
                    (0..4)
                        .map(|corner| colors[corner][channel] * weights[corner])
                        .sum()
                });
            }
        }
    }
    [0.0; 4]
}

#[cfg(test)]
mod tests {
    use super::inverse;
    use kurbo::Point;

    #[test]
    fn inverse_recovers_nonuniform_bilinear_coordinates() {
        let corners = [
            Point::new(2.0, 3.0),
            Point::new(20.0, 7.0),
            Point::new(5.0, 24.0),
            Point::new(29.0, 19.0),
        ];
        for (u, v) in [(0.1, 0.2), (0.7, 0.3), (0.9, 0.9)] {
            let point = corners[0]
                + u * (corners[1] - corners[0])
                + v * (corners[2] - corners[0])
                + u * v * (corners[3] - corners[2] - (corners[1] - corners[0]));
            let actual = inverse(point, corners).unwrap();
            assert!((actual.0 - u).abs() < 1e-12 && (actual.1 - v).abs() < 1e-12);
        }
    }

    #[test]
    fn collapsed_patch_has_no_sample() {
        assert!(
            inverse(
                Point::new(2.0, 0.0),
                [
                    Point::ZERO,
                    Point::new(4.0, 0.0),
                    Point::ZERO,
                    Point::new(8.0, 0.0)
                ]
            )
            .is_none()
        );
    }
}
