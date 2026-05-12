use crate::{
    IntoFFI, IntoRust, WuiEnv, ffi_computed, ffi_computed_ctor, ffi_reactive, reactive::WuiComputed,
};

use waterui::{Color, Signal};
use waterui_core::{Environment, resolve::Resolvable};
use waterui_graphics::color::ResolvedColor;

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

// Note: ffi_view! not used here because Color is a composite view (has body()) when wgpu is enabled.
// Native backends render Color through the normal View body path, not as a NativeView.

ffi_computed!(ResolvedColor, WuiResolvedColor);
ffi_computed_ctor!(ResolvedColor, WuiResolvedColor);

ffi_reactive!(Color, *mut WuiColor);

// `ResolvedColor` is a raw view (native fill) on all backends to avoid creating
// GPU surfaces for simple color blocks.
ffi_view!(ResolvedColor, WuiResolvedColor, resolved_color);

// JNI primitive support for Color (pointer treated as jlong)
#[cfg(feature = "android-jni")]
impl crate::jni::JniPrimitive for Color {
    type Jni = jni::sys::jlong;
    fn to_jni(self) -> Self::Jni {
        self.into_ffi() as Self::Jni
    }
    fn from_jni(val: Self::Jni) -> Self {
        unsafe { IntoRust::into_rust(val as *mut WuiColor) }
    }
}

// Generate JNI read/set for Color binding
crate::jni_binding_primitive!(Color, color);

// Generate JNI read for Color computed
crate::jni_computed_primitive!(Color, color);

#[derive(Debug, Clone)]
struct LinearResolvedColor {
    resolved: ResolvedColor,
}

impl Resolvable for LinearResolvedColor {
    type Resolved = ResolvedColor;
    fn resolve(&self, _env: &Environment) -> impl Signal<Output = Self::Resolved> {
        self.resolved
    }
}

/// Creates a new linear sRGBA color with optional HDR headroom.
///
/// `headroom` is an HDR scale factor where `0.0` means SDR and values above
/// `0.0` allow the renderer to apply an extended range multiplier.
///
/// # Safety
///
/// This function returns an owned pointer that must be dropped with
/// `waterui_drop_color` unless it is passed to a binding setter that consumes it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_color_from_linear_rgba_headroom(
    red: f32,
    green: f32,
    blue: f32,
    alpha: f32,
    headroom: f32,
) -> *mut WuiColor {
    let resolved = ResolvedColor {
        red,
        green,
        blue,
        opacity: alpha.clamp(0.0, 1.0),
        headroom: headroom.max(0.0),
    };
    Color::new(LinearResolvedColor { resolved }).into_ffi()
}

/// Creates a new linear sRGBA color (SDR only).
///
/// # Safety
///
/// This function returns an owned pointer that must be dropped with
/// `waterui_drop_color` unless it is passed to a binding setter that consumes it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_color_from_srgba(
    red: f32,
    green: f32,
    blue: f32,
    alpha: f32,
) -> *mut WuiColor {
    unsafe { waterui_color_from_linear_rgba_headroom(red, green, blue, alpha, 0.0) }
}

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
