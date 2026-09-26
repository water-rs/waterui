use waterui_graphics::cherenkov::Paint;
use waterui_graphics::{Gradient, GradientType};

use crate::{IntoFFI, WuiArray, color::WuiWorkingColor};

/// C ABI mirror of a gradient colour stop.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct WuiGradientStop {
    /// Position along the gradient, from 0 to 1.
    pub position: f32,
    /// The colour at this position.
    pub color: WuiWorkingColor,
}

/// C ABI mirror of a mesh gradient vertex in unit space.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct WuiMeshVertex {
    /// Unit-space x.
    pub x: f32,
    /// Unit-space y.
    pub y: f32,
    /// The vertex colour.
    pub color: WuiWorkingColor,
}

/// C ABI mirror of [`GradientType`], the discriminator for a gradient's shape.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub enum WuiGradientType {
    /// Linear gradient along a line.
    Linear = 0,
    /// Radial gradient from a center point.
    Radial = 1,
    /// Angular (conic) gradient around a center point.
    Angular = 2,
    /// 2D mesh gradient.
    Mesh = 3,
}

impl IntoFFI for GradientType {
    type FFI = WuiGradientType;

    fn into_ffi(self) -> Self::FFI {
        match self {
            Self::Linear => WuiGradientType::Linear,
            Self::Radial => WuiGradientType::Radial,
            Self::Angular => WuiGradientType::Angular,
            Self::Mesh => WuiGradientType::Mesh,
        }
    }
}

/// C ABI mirror of [`Gradient`], the backend-native gradient payload in unit
/// space. Backends scale it onto the view's bounds.
#[repr(C)]
#[derive(Debug)]
pub struct WuiGradient {
    /// Gradient kind (linear, radial, angular, or mesh).
    pub gradient_type: WuiGradientType,
    /// The colour stops; empty for a mesh.
    pub stops: WuiArray<WuiGradientStop>,
    /// Start point (linear) or center (radial/angular) x-coordinate.
    pub start_x: f32,
    /// Start point (linear) or center (radial/angular) y-coordinate.
    pub start_y: f32,
    /// End point (linear) x-coordinate.
    pub end_x: f32,
    /// End point (linear) y-coordinate.
    pub end_y: f32,
    /// Start radius (radial) or start angle in radians (angular).
    pub start_value: f32,
    /// End radius (radial) or end angle in radians (angular).
    pub end_value: f32,
    /// Mesh grid vertices per row; 0 unless the gradient is a mesh.
    pub mesh_columns: u32,
    /// Mesh grid vertices per column; 0 unless the gradient is a mesh.
    pub mesh_rows: u32,
    /// Mesh grid vertices, row by row; empty unless the gradient is a mesh.
    pub mesh_vertices: WuiArray<WuiMeshVertex>,
}

#[allow(clippy::cast_possible_truncation)]
impl IntoFFI for Gradient {
    type FFI = WuiGradient;

    fn into_ffi(self) -> Self::FFI {
        let f = |v: f64| v as f32;
        let stops = |stops: &[waterui_graphics::cherenkov::ColorStop]| {
            WuiArray::new(
                stops
                    .iter()
                    .map(|stop| WuiGradientStop {
                        position: stop.offset,
                        color: stop.color.into_ffi(),
                    })
                    .collect::<Vec<_>>(),
            )
        };
        let gradient_type = self.gradient_type().into_ffi();
        match self.paint() {
            Paint::Linear(linear) => WuiGradient {
                gradient_type,
                stops: stops(&linear.stops),
                start_x: f(linear.start.x),
                start_y: f(linear.start.y),
                end_x: f(linear.end.x),
                end_y: f(linear.end.y),
                start_value: 0.0,
                end_value: 0.0,
                mesh_columns: 0,
                mesh_rows: 0,
                mesh_vertices: WuiArray::new(Vec::new()),
            },
            Paint::Radial(radial) => WuiGradient {
                gradient_type,
                stops: stops(&radial.stops),
                start_x: f(radial.start_center.x),
                start_y: f(radial.start_center.y),
                end_x: f(radial.end_center.x),
                end_y: f(radial.end_center.y),
                start_value: f(radial.start_radius),
                end_value: f(radial.end_radius),
                mesh_columns: 0,
                mesh_rows: 0,
                mesh_vertices: WuiArray::new(Vec::new()),
            },
            Paint::Sweep(sweep) => WuiGradient {
                gradient_type,
                stops: stops(&sweep.stops),
                start_x: f(sweep.center.x),
                start_y: f(sweep.center.y),
                end_x: f(sweep.center.x),
                end_y: f(sweep.center.y),
                start_value: f(sweep.start_angle),
                end_value: f(sweep.end_angle),
                mesh_columns: 0,
                mesh_rows: 0,
                mesh_vertices: WuiArray::new(Vec::new()),
            },
            Paint::Mesh(mesh) => WuiGradient {
                gradient_type,
                stops: WuiArray::new(Vec::new()),
                start_x: 0.0,
                start_y: 0.0,
                end_x: 1.0,
                end_y: 1.0,
                start_value: 0.0,
                end_value: 0.0,
                mesh_columns: mesh.columns() + 1,
                mesh_rows: mesh.rows() + 1,
                mesh_vertices: WuiArray::new(
                    mesh.points()
                        .iter()
                        .zip(mesh.colors())
                        .map(|(point, color)| WuiMeshVertex {
                            x: f(point.x),
                            y: f(point.y),
                            color: (*color).into_ffi(),
                        })
                        .collect::<Vec<_>>(),
                ),
            },
            Paint::Solid(_) | Paint::Image(_) | Paint::Shader(_) => {
                unreachable!("a gradient view only carries gradient paints")
            }
        }
    }
}

// `Gradient` is a raw view rendered natively by platform backends.
ffi_view!(Gradient, WuiGradient, gradient);
