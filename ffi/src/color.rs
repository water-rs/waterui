use crate::IntoNullableFFI;

use waterui::{Color, core::color::Colorspace};

#[repr(C)]
pub enum WuiColorSpace {
    Srgb,
    P3,
    Invalid,
}

#[repr(C)]
pub struct WuiColor {
    color_space: WuiColorSpace,
    red: f32,
    yellow: f32,
    blue: f32,
    opacity: f32,
}

impl IntoNullableFFI for Color {
    type FFI = WuiColor;
    fn into_ffi(self) -> Self::FFI {
        match self.color_space() {
            Colorspace::P3 => {
                let (r, g, b, opacity) = self.p3();
                WuiColor {
                    color_space: WuiColorSpace::P3,
                    red: r,
                    yellow: g,
                    blue: b,
                    opacity,
                }
            }
            _ => {
                let (r, g, b, opacity) = self.rgba();
                WuiColor {
                    color_space: WuiColorSpace::Srgb,
                    red: r,
                    yellow: g,
                    blue: b,
                    opacity,
                }
            } // by default, we use sRGB
        }
    }

    fn null() -> Self::FFI {
        WuiColor {
            color_space: WuiColorSpace::Invalid,
            red: 0.0,
            yellow: 0.0,
            blue: 0.0,
            opacity: 0.0,
        }
    }
}
