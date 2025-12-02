use crate::{
    IntoFFI, IntoRust, WuiEnv, ffi_computed, ffi_computed_ctor, ffi_reactive, reactive::WuiComputed,
};

use waterui::Color;
use waterui_color::ResolvedColor;

opaque!(WuiColor, Color);

into_ffi!(
    ResolvedColor,
    pub struct WuiResolvedColor {
        red: f32,
        green: f32,
        blue: f32,
        opacity: f32,
        headroom: f32,
    }
);

impl IntoRust for WuiResolvedColor {
    type Rust = ResolvedColor;
    unsafe fn into_rust(self) -> Self::Rust {
        ResolvedColor {
            red: self.red,
            green: self.green,
            blue: self.blue,
            opacity: self.opacity,
            headroom: self.headroom,
        }
    }
}

ffi_view!(Color, *mut WuiColor, color);

ffi_computed!(ResolvedColor, WuiResolvedColor);
ffi_computed_ctor!(ResolvedColor, WuiResolvedColor);

ffi_reactive!(Color, *mut WuiColor);

/// Resolves a color in the given environment.
///
/// # Safety
///
/// Both `color` and `env` must be valid, non-null pointers to their respective types.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_resolve_color(
    color: *const WuiColor,
    env: *const WuiEnv,
) -> *mut WuiComputed<ResolvedColor> {
    unsafe {
        let color = &*color;
        let env = &*env;
        let resolved = color.resolve(env);
        resolved.into_ffi()
    }
}
