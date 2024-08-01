use core::any::TypeId;

use waterui::{AnyView, View, ViewExt};

use crate::{waterui_anyview, waterui_env, waterui_type_id, IntoFFI, IntoRust};

pub mod button;
pub mod metadata;
pub mod picker;
pub mod progress;
pub mod stack;
pub mod stepper;
pub mod text;
pub mod text_field;
pub mod toggle;

#[no_mangle]
unsafe extern "C" fn waterui_view_id(view: *const waterui_anyview) -> waterui_type_id {
    AnyView::type_id(&*view).into_ffi()
}

#[no_mangle]
extern "C" fn waterui_view_empty_id() -> waterui_type_id {
    TypeId::of::<()>().into_ffi()
}

#[no_mangle]
unsafe extern "C" fn waterui_view_body(
    view: *mut waterui_anyview,
    env: *const waterui_env,
) -> *mut waterui_anyview {
    view.into_rust().body(&*env).anyview().into_ffi()
}
