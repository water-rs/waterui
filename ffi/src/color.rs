use crate::{impl_binding, impl_computed, IntoFFI, WuiEnv};

use nami::Computed;
use waterui::{Color};
use waterui_color::ResolvedColor;

ffi_type!(WuiColor, Color, waterui_drop_color);

pub enum WuiColorspace {
    Srgb,
    DisplayP3,
}

// colorspace: extended srgb
#[repr(C)]
pub struct WuiResolvedColor {
    red: f32,
    green: f32,
    blue: f32,
    opacity: f32,
}

ffi_struct!(ResolvedColor, WuiResolvedColor, red, green, blue, opacity);

impl_computed!(
    ResolvedColor,
    WuiResolvedColor,
    waterui_read_computed_resolved_color,
    waterui_watch_computed_resolved_color,
    waterui_drop_resolved_color_watcher_guard
);

impl_binding!(
    Color,
    *mut WuiColor,
    waterui_binding_read_color,
    waterui_binding_set_color,
    waterui_binding_watch_color,
    waterui_drop_color_watcher_guard
);

impl_computed!(
    Color,
    *mut WuiColor,
    waterui_read_computed_color,
    waterui_watch_computed_color,
    waterui_drop_computed_color
);




/// Resolves a color in the given environment.
///
/// # Safety
///
/// Both `color` and `env` must be valid, non-null pointers to their respective types.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_resolve_color(color:*const WuiColor,env:*const WuiEnv) -> *mut Computed<ResolvedColor> {
    unsafe {
        let color = &*color;
        let env = &*env;
        let resolved = color.resolve(env);
        resolved.into_ffi()
    }
}