use crate::{IntoFFI, IntoRust, WuiEnv, ffi_computed, ffi_computed_ctor, reactive::WuiComputed};

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

ffi_computed!(ResolvedColor, WuiResolvedColor);
ffi_computed_ctor!(ResolvedColor, WuiResolvedColor);

crate::ffi_binding!(Color, *mut WuiColor, color);
#[cfg(feature = "c-api")]
crate::ffi_watcher!(Color, *mut WuiColor, color);

// `ResolvedColor` is a raw view (native fill) on all backends to avoid creating
// GPU surfaces for simple color blocks.
ffi_view!(ResolvedColor, WuiResolvedColor, resolved_color);

/// Consumes a semantic color view and returns its owned resolvable color handle.
///
/// # Safety
///
/// `view` must own a native `Color` view and must not be used again.
#[cfg(feature = "c-api")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_force_as_color(view: *mut crate::WuiAnyView) -> *mut WuiColor {
    // SAFETY: the caller contract makes `view` a valid owning handle consumed here.
    let any: waterui::AnyView = unsafe { IntoRust::into_rust(view) };
    // SAFETY: the same contract guarantees the erased value is a `Native<Color>`.
    let native = unsafe { *any.downcast_unchecked::<waterui_core::Native<Color>>() };
    native.into_ffi()
}

/// Returns the native semantic color view type id.
#[cfg(feature = "c-api")]
#[unsafe(no_mangle)]
pub extern "C" fn waterui_color_id() -> crate::WuiTypeId {
    crate::WuiTypeId::of::<waterui_core::Native<Color>>()
}

#[cfg(feature = "android-jni")]
#[unsafe(no_mangle)]
extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_colorId<'local>(
    mut env: crate::jni::JNIEnv<'local>,
    _class: crate::jni::JClass<'local>,
) -> crate::jni::jobject {
    crate::jni::with_env(&mut env, |env| {
        crate::jni::type_id_to_java(env, crate::WuiTypeId::of::<waterui_core::Native<Color>>())
            .into_raw()
    })
}

/// Consumes a semantic color view and returns its owned resolvable color handle.
///
/// # Safety
///
/// `view_ptr` must own a native `Color` view and must not be used again.
#[cfg(feature = "android-jni")]
#[unsafe(no_mangle)]
unsafe extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_forceAsColor<'local>(
    _env: crate::jni::JNIEnv<'local>,
    _class: crate::jni::JClass<'local>,
    view_ptr: crate::jni::jlong,
) -> crate::jni::jlong {
    let view = view_ptr as *mut crate::WuiAnyView;
    // SAFETY: the caller contract makes `view_ptr` a valid owning handle consumed here.
    let any: waterui::AnyView = unsafe { IntoRust::into_rust(view) };
    // SAFETY: the same contract guarantees the erased value is a `Native<Color>`.
    let native = unsafe { *any.downcast_unchecked::<waterui_core::Native<Color>>() };
    native.into_ffi() as crate::jni::jlong
}

// JNI primitive support for Color (pointer treated as jlong)
#[cfg(feature = "android-jni")]
impl crate::jni::JniPrimitive for Color {
    type Jni = jni::sys::jlong;
    fn to_jni(self) -> Self::Jni {
        self.into_ffi() as Self::Jni
    }
    fn from_jni(val: Self::Jni) -> Self {
        // SAFETY: `JniPrimitive` round-trips one value: `val` is the `jlong` produced
        // by `to_jni` above, which is an owning `WuiColor` pointer, and reclaiming it
        // here consumes it exactly once.
        unsafe { IntoRust::into_rust(val as *mut WuiColor) }
    }
}

// Generate JNI read/set for Color binding
crate::jni_binding_primitive!(Color, color);

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
    let mut resolved =
        ResolvedColor::from_srgb(waterui_graphics::color::Srgb::new(red, green, blue));
    resolved.opacity = alpha.clamp(0.0, 1.0);
    Color::new(LinearResolvedColor { resolved }).into_ffi()
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
    // SAFETY: the caller contract requires `color` and `env` to be valid handles that
    // stay alive for this call; both are only borrowed here.
    unsafe {
        let color = &*color;
        let env = &*env;
        let resolved = color.resolve(env);
        resolved.into_ffi()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgba_constructor_decodes_transfer_function() {
        // SAFETY: the returned owning handle is consumed exactly once below.
        let pointer = unsafe { waterui_color_from_srgba(0.5, 0.5, 0.5, 0.25) };
        // SAFETY: `pointer` is the valid owning handle returned above.
        let color: Color = unsafe { IntoRust::into_rust(pointer) };
        let resolved = color.resolve(&Environment::new()).get();

        assert!((resolved.red - 0.214_041_14).abs() < 1.0e-6);
        assert!((resolved.green - 0.214_041_14).abs() < 1.0e-6);
        assert!((resolved.blue - 0.214_041_14).abs() < 1.0e-6);
        assert!((resolved.opacity - 0.25).abs() < f32::EPSILON);
    }
}
