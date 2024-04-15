use crate::ffi::{Action, AnyView};

#[repr(C)]
pub struct Button {
    label: AnyView,
    action: Action,
}

impl From<crate::component::Button> for Button {
    fn from(value: crate::component::Button) -> Self {
        Self {
            label: value._label.into(),
            action: value._action.into(),
        }
    }
}

impl_view!(Button, waterui_view_force_as_button, waterui_view_button_id);
