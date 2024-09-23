use core::any::TypeId;

use waterui::{component::Native, AnyView, Binding, View, ViewExt};

use crate::{waterui_anyview, waterui_env, waterui_type_id, IntoFFI, IntoRust};

pub mod button;
pub mod dynamic;
pub mod metadata;
pub mod navigation;
pub mod picker;
pub mod progress;
pub mod stack;
pub mod stepper;
pub mod tabs;
pub mod text;
pub mod text_field;
pub mod toggle;
pub mod with_env;
ffi_type!(
    waterui_binding_id,
    Binding<waterui::utils::Id>,
    waterui_drop_binding_id
);

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
    env: *mut waterui_env,
) -> *mut waterui_anyview {
    view.into_rust().body(env.into_rust()).anyview().into_ffi()
}

impl<T: IntoFFI> IntoFFI for Native<T> {
    type FFI = T::FFI;
    fn into_ffi(self) -> Self::FFI {
        self.0.into_ffi()
    }
}
