use crate::reactive::WuiComputed;
use crate::{IntoFFI, IntoRust, WuiAnyView};
use waterui::AnyView;
use waterui_core::Metadata;
use waterui_graphics::filter_view::{
    AppliedFilter, Blur, FilteredView, Reactive, blur_from_radius_signal,
};

/// FFI representation of `FilteredView<Blur>`.
#[repr(C)]
pub struct WuiFilteredBlur {
    /// Child content pointer consumed by native backend.
    pub content: *mut WuiAnyView,
    /// Reactive blur radius pointer.
    pub radius: *mut WuiComputed<f32>,
}

impl IntoFFI for FilteredView<Blur> {
    type FFI = WuiFilteredBlur;

    fn into_ffi(self) -> Self::FFI {
        let radius = self.filter().radius_signal().0.clone();
        let content = self.into_content();
        WuiFilteredBlur {
            content: content.into_ffi(),
            radius: radius.into_ffi(),
        }
    }
}

/// Force-casts `AnyView` to `FilteredView<Blur>`.
///
/// # Safety
/// Caller must guarantee `view` points to a `FilteredView<Blur>`.
#[cfg(feature = "c-api")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_force_as_filtered_blur(view: *mut WuiAnyView) -> WuiFilteredBlur {
    let any: AnyView = unsafe { IntoRust::into_rust(view) };
    let filtered = unsafe { *any.downcast_unchecked::<FilteredView<Blur>>() };
    filtered.into_ffi()
}

/// Expands filtered blur into the generic fallback `Metadata<AppliedFilter>` node.
///
/// This consumes `content` and `radius`, rebuilding the fallback subtree exactly
/// as `FilteredView<Blur>::body()`.
///
/// # Safety
/// `content` must be a valid `WuiAnyView*`, `radius` must be a valid `WuiComputed<f32>*`.
#[cfg(feature = "c-api")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_filtered_blur_expand(
    content: *mut WuiAnyView,
    radius: *mut WuiComputed<f32>,
) -> *mut WuiAnyView {
    let content: AnyView = unsafe { IntoRust::into_rust(content) };
    let radius = unsafe { IntoRust::into_rust(radius) }
        .expect("waterui_filtered_blur_expand: radius must not be null");
    let blur = blur_from_radius_signal(Reactive(radius.0));
    let fallback = Metadata::new(content, AppliedFilter::new(blur));
    AnyView::new(fallback).into_ffi()
}

#[cfg(feature = "android-jni")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_forceAsFilteredBlur<
    'local,
>(
    mut env: crate::jni::JNIEnv<'local>,
    _class: crate::jni::JClass<'local>,
    view_ptr: crate::jni::jlong,
) -> crate::jni::jobject {
    use crate::jni::convert::jlong_to_ptr_mut;

    let view_ptr: *mut WuiAnyView = unsafe { jlong_to_ptr_mut(view_ptr) };
    let any: AnyView = unsafe { IntoRust::into_rust(view_ptr) };
    let ffi_struct: WuiFilteredBlur =
        unsafe { (*any.downcast_unchecked::<FilteredView<Blur>>()).into_ffi() };
    crate::jni::convert::struct_to_java(&mut env, &ffi_struct).into_raw()
}

#[cfg(feature = "android-jni")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_filteredBlurExpand<'local>(
    _env: crate::jni::JNIEnv<'local>,
    _class: crate::jni::JClass<'local>,
    content_ptr: crate::jni::jlong,
    radius_ptr: crate::jni::jlong,
) -> crate::jni::jlong {
    use crate::jni::convert::jlong_to_ptr_mut;

    let content = unsafe { jlong_to_ptr_mut(content_ptr) };
    let radius = unsafe { jlong_to_ptr_mut(radius_ptr) };
    unsafe { waterui_filtered_blur_expand(content, radius) as crate::jni::jlong }
}
