use waterui::{
    AnyView,
    views::{AnyViews, Views},
};

use crate::{IntoFFI, WuiAnyView, WuiId, impl_computed};

ffi_type!(WuiAnyViews, AnyViews<AnyView>, waterui_drop_anyviews);

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

impl_computed!(
    AnyViews<AnyView>,
    *mut WuiAnyViews,
    waterui_read_computed_views,
    waterui_watch_computed_views,
    waterui_drop_computed_views
);
