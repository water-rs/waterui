pub mod env;
pub mod frame;
pub mod padding;
use crate::{waterui_anyview, IntoFFI};
use waterui::component::metadata::Metadata;
#[repr(C)]
pub struct waterui_metadata<T> {
    content: *mut waterui_anyview,
    value: T,
}

impl<T: IntoFFI> IntoFFI for Metadata<T> {
    type FFI = waterui_metadata<T::FFI>;
    fn into_ffi(self) -> Self::FFI {
        waterui_metadata {
            content: self.content.into_ffi(),
            value: self.value.into_ffi(),
        }
    }
}
