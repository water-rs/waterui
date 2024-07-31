use crate::{array::waterui_array, ffi_view, waterui_anyview, IntoFFI};

use waterui::component::stack::{Stack, StackMode};

#[repr(C)]
pub enum waterui_stack_mode {
    AUTO,
    VERTICAL,
    HORIZONTAL,
    LAYERED,
}

impl IntoFFI for StackMode {
    type FFI = waterui_stack_mode;
    fn into_ffi(self) -> Self::FFI {
        match self {
            StackMode::Auto => waterui_stack_mode::AUTO,
            StackMode::Vertical => waterui_stack_mode::VERTICAL,
            StackMode::Horizonal => waterui_stack_mode::HORIZONTAL,
            StackMode::Layered => waterui_stack_mode::LAYERED,
        }
    }
}

#[repr(C)]
pub struct waterui_stack {
    contents: waterui_array<*mut waterui_anyview>,
    mode: waterui_stack_mode,
}

impl IntoFFI for Stack {
    type FFI = waterui_stack;
    fn into_ffi(self) -> Self::FFI {
        waterui_stack {
            contents: self._contents.into_ffi(),
            mode: self._mode.into_ffi(),
        }
    }
}

ffi_view!(
    Stack,
    waterui_stack,
    waterui_view_force_as_stack,
    waterui_view_stack_id
);
