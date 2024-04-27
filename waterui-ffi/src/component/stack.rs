use crate::{array::waterui_array, ffi_view, waterui_anyview, IntoFFI};

use alloc::vec::Vec;
use waterui::component::stack::Stack;
pub use waterui::component::stack::StackMode as waterui_stack_mode;

#[repr(C)]
pub struct waterui_stack {
    views: waterui_array<waterui_anyview>,
    mode: waterui_stack_mode,
}

impl IntoFFI for Stack {
    type FFI = waterui_stack;
    fn into_ffi(self) -> Self::FFI {
        waterui_stack {
            views: self
                ._views
                .into_iter()
                .map(waterui_anyview)
                .collect::<Vec<_>>()
                .into_ffi(),
            mode: self._mode,
        }
    }
}

ffi_view!(
    Stack,
    waterui_stack,
    waterui_view_force_as_stack,
    waterui_view_stack_id
);
