use waterui_graphics::{GradientType, ResolvedGradient, ResolvedGradientStop};

use crate::{IntoFFI, WuiArray, color::WuiResolvedColor};

into_ffi!(
    ResolvedGradientStop,
    pub struct WuiResolvedGradientStop {
        position: f32,
        color: WuiResolvedColor,
    }
);

/// C ABI mirror of [`GradientType`], the discriminator for a resolved gradient's shape.
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

/// C ABI mirror of [`ResolvedGradient`], the backend-native gradient payload
/// produced once a gradient's dynamic inputs have been resolved.
#[repr(C)]
#[derive(Debug)]
pub struct WuiResolvedGradient {
    /// Gradient kind (linear, radial, angular, or mesh).
    pub gradient_type: WuiGradientType,
    /// The gradient's resolved color stops.
    pub stops: WuiArray<WuiResolvedGradientStop>,
    /// Start point (linear) or center (radial/angular) x-coordinate.
    pub start_x: f32,
    /// Start point (linear) or center (radial/angular) y-coordinate.
    pub start_y: f32,
    /// End point (linear) x-coordinate.
    pub end_x: f32,
    /// End point (linear) y-coordinate.
    pub end_y: f32,
    /// Start radius (radial) or start angle (angular).
    pub start_value: f32,
    /// End radius (radial) or end angle (angular).
    pub end_value: f32,
}

impl IntoFFI for ResolvedGradient {
    type FFI = WuiResolvedGradient;

    fn into_ffi(self) -> Self::FFI {
        let stops = self
            .stops
            .into_iter()
            .map(IntoFFI::into_ffi)
            .collect::<Vec<WuiResolvedGradientStop>>();

        WuiResolvedGradient {
            gradient_type: self.gradient_type.into_ffi(),
            stops: WuiArray::new(stops),
            start_x: self.start_point[0],
            start_y: self.start_point[1],
            end_x: self.end_point[0],
            end_y: self.end_point[1],
            start_value: self.start_value,
            end_value: self.end_value,
        }
    }
}

// `ResolvedGradient` is a raw view rendered natively by platform backends.
ffi_view!(ResolvedGradient, WuiResolvedGradient, resolved_gradient);
