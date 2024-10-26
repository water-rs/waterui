use crate::{action::waterui_action, waterui_anyview};
use waterui::component::button::ButtonConfig;

#[repr(C)]
pub struct waterui_button {
    label: *mut waterui_anyview,
    action: *mut waterui_action,
}

into_ffi!(ButtonConfig, waterui_button, label, action);

native_view!(
    ButtonConfig,
    waterui_button,
    waterui_view_force_as_button,
    waterui_view_button_id
);
