use crate::{IntoFFI, WuiStr, WuiTypeId};

pub mod layout;

impl<T: IntoFFI + waterui_core::NativeView> IntoFFI for waterui_core::Native<T> {
    type FFI = T::FFI;
    fn into_ffi(self) -> Self::FFI {
        IntoFFI::into_ffi(self.0)
    }
}

pub mod button;

ffi_view!(waterui::Str, WuiStr, plain);
pub mod lazy;

pub mod link;

pub mod text;

/// Form component FFI bindings
pub mod form;

/// Navigation component FFI bindings
pub mod navigation;

/// Media component FFI bindings
pub mod media;

pub mod dynamic;

pub mod list;

pub mod table;

/// Returns the type ID for empty views as a 128-bit value.
#[unsafe(no_mangle)]
pub extern "C" fn waterui_empty_id() -> WuiTypeId {
    WuiTypeId::of::<()>()
}

pub mod progress;

// TODO: Re-enable when waterui_graphics types are implemented
// pub mod graphics;
