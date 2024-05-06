use core::any::TypeId;

use waterui::component::each::RawEach;

use crate::{waterui_anyview, waterui_type_id, IntoFFI, IntoRust};

ffi_type!(waterui_each, RawEach);

#[no_mangle]
unsafe extern "C" fn waterui_view_force_as_each(view: *mut waterui_anyview) -> *mut waterui_each {
    view.into_rust().downcast_unchecked::<RawEach>().into_ffi()
}

#[no_mangle]
extern "C" fn waterui_view_each_id() -> waterui_type_id {
    TypeId::of::<RawEach>().into_ffi()
}

#[no_mangle]
pub unsafe extern "C" fn waterui_each_id(each: *mut waterui_each, index: usize) -> usize {
    (*each).id(index)
}

#[no_mangle]
pub unsafe extern "C" fn waterui_each_pull(
    each: *mut waterui_each,
    index: usize,
) -> *mut waterui_anyview {
    (*each).pull(index).into_ffi()
}

#[no_mangle]
pub unsafe extern "C" fn waterui_each_len(each: *const waterui_each) -> usize {
    (*each).len()
}
