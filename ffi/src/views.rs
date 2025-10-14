use waterui::{
    AnyView,
    views::{AnyViews, Views},
};

use crate::{IntoFFI, WuiAnyView, WuiId, impl_computed};

ffi_type!(WuiAnyViews, AnyViews<AnyView>, waterui_drop_anyviews);

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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_anyviews_get_view(
    anyview: *const WuiAnyViews,
    index: usize,
) -> *mut WuiAnyView {
    unsafe { (&*anyview).get_view(index).into_ffi() }
}

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
