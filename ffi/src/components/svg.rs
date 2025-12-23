//! FFI bindings for the Svg component.

use crate::color::WuiColor;
use crate::{IntoFFI, WuiStr};
use waterui_graphics::Svg;

/// FFI representation of the Svg component.
///
/// The SVG content can be either path data (d attribute) or full SVG markup.
/// Native backends parse and render using platform-native vector graphics.
#[repr(C)]
pub struct WuiSvg {
    /// SVG content (path data or full SVG markup).
    pub content: WuiStr,
    /// Intrinsic width (0 means unspecified).
    pub width: f32,
    /// Intrinsic height (0 means unspecified).
    pub height: f32,
    /// Optional tint color (null means no tint, use original colors).
    pub tint: *mut WuiColor,
}

impl IntoFFI for Svg {
    type FFI = WuiSvg;
    fn into_ffi(self) -> Self::FFI {
        WuiSvg {
            content: self.content.into_ffi(),
            width: self.width.unwrap_or(0.0),
            height: self.height.unwrap_or(0.0),
            tint: self.tint.into_ffi(),
        }
    }
}

ffi_view!(Svg, WuiSvg, svg);
