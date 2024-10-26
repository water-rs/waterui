use crate::{array::waterui_array, ffi_view, waterui_anyview};

use waterui::component::layout::stack::{Stack, StackMode};

pub type waterui_stack_mode = StackMode;

#[repr(C)]
pub struct waterui_stack {
    contents: waterui_array<*mut waterui_anyview>,
    mode: waterui_stack_mode,
}

into_ffi!(Stack, waterui_stack, contents, mode);

ffi_safe!(waterui_stack_mode);

ffi_view!(
    Stack,
    waterui_stack,
    waterui_view_force_as_stack,
    waterui_view_stack_id
);
