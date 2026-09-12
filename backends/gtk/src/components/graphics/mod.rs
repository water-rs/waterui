//! GTK widget implementations for `WaterUI` graphics components.

use gtk4::gsk;
use kurbo::{BezPath, PathEl};

pub mod clip_shape_widget;
pub mod color;
pub mod gpu_surface;
pub mod gradient;
pub mod picture;
pub mod shape;

/// Converts a resolved path into the form GSK draws, fills and clips with.
///
/// The elements are exactly what [`crate::shape_path::bez_path`] produces —
/// arcs have already become cubics there — so this is a transcription, not a
/// second interpretation of the shape.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    reason = "GSK geometry is f32 while the resolved shape geometry is f64"
)]
pub fn gsk_path(path: &BezPath) -> gsk::Path {
    let builder = gsk::PathBuilder::new();
    for element in path.elements() {
        match *element {
            PathEl::MoveTo(p) => builder.move_to(p.x as f32, p.y as f32),
            PathEl::LineTo(p) => builder.line_to(p.x as f32, p.y as f32),
            PathEl::QuadTo(c, p) => builder.quad_to(c.x as f32, c.y as f32, p.x as f32, p.y as f32),
            PathEl::CurveTo(c1, c2, p) => builder.cubic_to(
                c1.x as f32,
                c1.y as f32,
                c2.x as f32,
                c2.y as f32,
                p.x as f32,
                p.y as f32,
            ),
            PathEl::ClosePath => builder.close(),
        }
    }
    builder.to_path()
}
