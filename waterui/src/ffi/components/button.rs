use crate::component::button::RawButton;
use crate::ffi::{Action, AnyView};
#[repr(C)]
pub struct Button {
    label: AnyView,
    action: Action,
}

impl From<RawButton> for Button {
    fn from(value: RawButton) -> Self {
        Self {
            label: value._label.into(),
            action: value._action.into(),
        }
    }
}

impl_view!(
    RawButton,
    Button,
    waterui_view_force_as_button,
    waterui_view_button_id
);
