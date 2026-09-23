use waterui_graphics::{GradientType, ResolvedGradient, ResolvedGradientStop};

use crate::{IntoFFI, WuiArray, color::WuiResolvedColor};

into_ffi!(
    ResolvedGradientStop,
    pub struct WuiResolvedGradientStop {
        position: f32,
        color: WuiResolvedColor,
    }
);

#[repr(C)]
#[derive(Clone, Copy)]
pub enum WuiGradientType {
    Linear = 0,
    Radial = 1,
    Angular = 2,
    Mesh = 3,
}

impl IntoFFI for GradientType {
    type FFI = WuiGradientType;

    fn into_ffi(self) -> Self::FFI {
        match self {
            GradientType::Linear => WuiGradientType::Linear,
            GradientType::Radial => WuiGradientType::Radial,
            GradientType::Angular => WuiGradientType::Angular,
            GradientType::Mesh => WuiGradientType::Mesh,
        }
    }
}

#[repr(C)]
pub struct WuiResolvedGradient {
    pub gradient_type: WuiGradientType,
    pub stops: WuiArray<WuiResolvedGradientStop>,
    pub start_x: f32,
    pub start_y: f32,
    pub end_x: f32,
    pub end_y: f32,
    pub start_value: f32,
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
