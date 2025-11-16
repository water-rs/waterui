use waterui::{
    AnyView,
    views::{AnyViews, Views},
};

use crate::{IntoFFI, WuiAnyView, ffi_computed, ffi_watcher_ctor, id::WuiId};

opaque!(WuiAnyViews, AnyViews<AnyView>, anyviews);

/// Gets the ID of a view at the specified index.
///
/// # Safety
/// The caller must ensure that `anyviews` is a valid pointer and `index` is within bounds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_anyviews_get_id(
    anyviews: *const WuiAnyViews,
    index: usize,
) -> WuiId {
    unsafe {
        (&*anyviews)
            .get_id(index)
            .expect("Out of bound")
            .into_inner()
            .into_ffi()
    }
}

/// Gets a view at the specified index.
///
/// # Safety
/// The caller must ensure that `anyview` is a valid pointer and `index` is within bounds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_anyviews_get_view(
    anyview: *const WuiAnyViews,
    index: usize,
) -> *mut WuiAnyView {
    unsafe { (&*anyview).get_view(index).into_ffi() }
}

/// Gets the number of views in the collection.
///
/// # Safety
/// The caller must ensure that `anyviews` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_anyviews_len(anyviews: *const WuiAnyViews) -> usize {
    unsafe { (&*anyviews).len() }
}

ffi_computed!(AnyViews<AnyView>, *mut WuiAnyViews, views);
ffi_watcher_ctor!(AnyViews<AnyView>, *mut WuiAnyViews, views);
