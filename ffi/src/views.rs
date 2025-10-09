use crate::{IntoFFI, WuiAnyView, WuiId};
use core::ptr::null_mut;
use core::{ffi::c_void, num::NonZeroI32};
use waterui::AnyView;
use waterui::views::AnyViews;
use waterui_core::id::Id as CoreId;
use waterui_core::views::Views;

ffi_type!(WuiAnyViews, AnyViews<AnyView>, waterui_drop_any_views);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_any_views_len(views: *const WuiAnyViews) -> usize {
    if views.is_null() {
        return 0;
    }
    unsafe { (*views).len() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_any_views_get_view(
    views: *const WuiAnyViews,
    index: usize,
) -> *mut WuiAnyView {
    if views.is_null() {
        return null_mut();
    }
    match unsafe { (*views).get_view(index) } {
        Some(view) => IntoFFI::into_ffi(view),
        None => null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_any_views_get_id(
    views: *const WuiAnyViews,
    index: usize,
) -> WuiId {
    // Use 1-based indices to guarantee we return a non-zero identifier.
    let fallback = (index as i32) + 1;
    let fallback_id = CoreId::try_from(fallback).unwrap();
    if views.is_null() {
        return IntoFFI::into_ffi(fallback_id);
    }

    match unsafe { (*views).get_id(index) } {
        Some(id) => {
            let value = id.into_inner().get();
            let candidate = i32::try_from(value)
                .ok()
                .and_then(NonZeroI32::new)
                .map(CoreId::from);
            IntoFFI::into_ffi(candidate.unwrap_or(fallback_id))
        }
        None => IntoFFI::into_ffi(fallback_id),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_any_views_opaque(value: *mut c_void) {
    unsafe {
        waterui_drop_any_views(value.cast());
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_any_views_len_opaque(views: *const c_void) -> usize {
    unsafe { waterui_any_views_len(views.cast()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_any_views_get_view_opaque(
    views: *const c_void,
    index: usize,
) -> *mut WuiAnyView {
    unsafe { waterui_any_views_get_view(views.cast(), index) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_any_views_get_id_opaque(
    views: *const c_void,
    index: usize,
) -> WuiId {
    unsafe { waterui_any_views_get_id(views.cast(), index) }
}
