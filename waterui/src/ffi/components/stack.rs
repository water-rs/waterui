use crate::component::stack::StackMode;

#[repr(C)]
pub struct Stack {
    views: Views,
    mode: StackMode,
}

impl From<crate::component::Stack> for Stack {
    fn from(value: crate::component::Stack) -> Self {
        Self {
            views: value._views.into(),
            mode: value._mode,
        }
    }
}

impl_view!(Stack, waterui_view_force_as_stack, waterui_view_stack_id);

impl_array!(Views, crate::component::AnyView, crate::ffi::AnyView);
