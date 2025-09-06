use crate::{IntoFFI, WuiTypeId, str::WuiStr};

pub mod layout;

impl<T: IntoFFI> IntoFFI for waterui::component::Native<T> {
    type FFI = T::FFI;
    fn into_ffi(self) -> Self::FFI {
        IntoFFI::into_ffi(self.0)
    }
}

ffi_view!(waterui::component::divder::Divider, waterui_divider_id);

pub mod button;

ffi_view!(
    waterui::Str,
    *mut WuiStr,
    waterui_force_as_label,
    waterui_label_id
);

pub mod link;

pub mod text;

/// Form component FFI bindings
pub mod form;

/// Navigation component FFI bindings
pub mod navigation;

/// Media component FFI bindings
pub mod media;

pub mod dynamic;

#[unsafe(no_mangle)]
pub extern "C" fn waterui_empty_id() -> WuiTypeId {
    core::any::TypeId::of::<()>().into_ffi()
}

#[unsafe(no_mangle)]
pub extern "C" fn waterui_spacer_id() -> WuiTypeId {
    core::any::TypeId::of::<waterui::component::layout::spacer::Spacer>().into_ffi()
}
