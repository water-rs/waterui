use crate::{IntoFFI, WuiAnyView, WuiId};
use alloc::rc::Rc;
use core::ffi::c_void;
use waterui::AnyView;
use waterui::views::AnyViews;
use waterui_core::id::Id;
use waterui_core::views::Views;

fn convert_id(value: usize, context: &str) -> Id {
    let raw = i32::try_from(value)
        .unwrap_or_else(|_| panic!("{context}: identifier {value} exceeds i32 range"));
    Id::try_from(raw).unwrap_or_else(|_| panic!("{context}: identifier must be non-zero"))
}

ffi_type!(WuiAnyViews, AnyViews<AnyView>, waterui_drop_any_views);
ffi_type!(
    WuiSharedAnyViews,
    Rc<AnyViews<AnyView>>,
    waterui_drop_shared_any_views
);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_any_views_len(views: *const WuiAnyViews) -> usize {
    assert!(
        !views.is_null(),
        "waterui_any_views_len: received null pointer"
    );
    unsafe { (*views).len() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_any_views_get_view(
    views: *const WuiAnyViews,
    index: usize,
) -> *mut WuiAnyView {
    assert!(
        !views.is_null(),
        "waterui_any_views_get_view: received null pointer"
    );
    let views = unsafe { &*views };
    let view = views.get_view(index).unwrap_or_else(|| {
        panic!(
            "waterui_any_views_get_view: index {index} out of range (len = {})",
            views.len()
        )
    });
    IntoFFI::into_ffi(view)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_any_views_get_id(
    views: *const WuiAnyViews,
    index: usize,
) -> WuiId {
    assert!(
        !views.is_null(),
        "waterui_any_views_get_id: received null pointer"
    );
    let views = unsafe { &*views };
    let id = views.get_id(index).unwrap_or_else(|| {
        panic!(
            "waterui_any_views_get_id: index {index} out of range (len = {})",
            views.len()
        )
    });
    let value = id.into_inner().get();
    IntoFFI::into_ffi(convert_id(value, "waterui_any_views_get_id"))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_shared_any_views_len(views: *const WuiSharedAnyViews) -> usize {
    assert!(
        !views.is_null(),
        "waterui_shared_any_views_len: received null pointer"
    );
    unsafe { (*views).0.len() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_shared_any_views_get_view(
    views: *const WuiSharedAnyViews,
    index: usize,
) -> *mut WuiAnyView {
    assert!(
        !views.is_null(),
        "waterui_shared_any_views_get_view: received null pointer"
    );
    let views = unsafe { &*views };
    let view = views.0.get_view(index).unwrap_or_else(|| {
        panic!(
            "waterui_shared_any_views_get_view: index {index} out of range (len = {})",
            views.0.len()
        )
    });
    IntoFFI::into_ffi(view)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_shared_any_views_get_id(
    views: *const WuiSharedAnyViews,
    index: usize,
) -> WuiId {
    assert!(
        !views.is_null(),
        "waterui_shared_any_views_get_id: received null pointer"
    );
    let views = unsafe { &*views };
    let id = views.0.get_id(index).unwrap_or_else(|| {
        panic!(
            "waterui_shared_any_views_get_id: index {index} out of range (len = {})",
            views.0.len()
        )
    });
    let value = id.into_inner().get();
    IntoFFI::into_ffi(convert_id(value, "waterui_shared_any_views_get_id"))
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_shared_any_views_len_opaque(views: *const c_void) -> usize {
    unsafe { waterui_shared_any_views_len(views.cast()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_shared_any_views_get_view_opaque(
    views: *const c_void,
    index: usize,
) -> *mut WuiAnyView {
    unsafe { waterui_shared_any_views_get_view(views.cast(), index) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_shared_any_views_get_id_opaque(
    views: *const c_void,
    index: usize,
) -> WuiId {
    unsafe { waterui_shared_any_views_get_id(views.cast(), index) }
}
