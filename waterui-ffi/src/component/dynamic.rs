use core::ptr::read;

use waterui::component::Dynamic;

use crate::{closure::waterui_fn, waterui_anyview, IntoFFI};

ffi_type!(waterui_dynamic_view, Dynamic);

ffi_view!(
    Dynamic,
    *mut waterui_dynamic_view,
    waterui_view_force_as_dynamic,
    waterui_view_dynamic_id
);

#[no_mangle]
unsafe extern "C" fn waterui_dynamic_view_connect(
    dyanmic: *mut waterui_dynamic_view,
    f: waterui_fn<*mut waterui_anyview>,
) {
    read(dyanmic).0.connect(move |view| {
        f.call(view.into_ffi());
    });
}
