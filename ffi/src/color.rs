use crate::{IntoFFI, WuiEnv, ffi_computed, ffi_reactive, reactive::WuiComputed};

use waterui::Color;
use waterui_color::ResolvedColor;

opaque!(WuiColor, Color);

pub enum WuiColorspace {
    Srgb,
    DisplayP3,
    Oklch,
}

into_ffi!(
    ResolvedColor,
    pub struct WuiResolvedColor {
        red: f32,
        green: f32,
        blue: f32,
        opacity: f32,
    }
);

ffi_view!(Color, *mut WuiColor);

ffi_computed!(ResolvedColor, WuiResolvedColor);

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
