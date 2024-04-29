use crate::{ffi_view, IntoFFI};

use alloc::boxed::Box;
use waterui::{
    component::stack::{Stack, StackMode},
    view::Views,
    AnyView,
};

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

ffi_type!(waterui_anyviews, Box<dyn Views<Item = AnyView>>);

#[repr(C)]
pub struct waterui_stack {
    views: *mut waterui_anyviews,
    mode: waterui_stack_mode,
}

impl IntoFFI for Stack {
    type FFI = waterui_stack;
    fn into_ffi(self) -> Self::FFI {
        waterui_stack {
            views: self._views.into_ffi(),
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
