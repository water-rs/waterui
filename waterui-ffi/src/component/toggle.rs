use crate::{binding::waterui_binding_bool, waterui_anyview};

use waterui::component::form::toggle::ToggleConfig;

#[repr(C)]
pub struct waterui_toggle {
    label: *mut waterui_anyview,
    toggle: *const waterui_binding_bool,
}

into_ffi!(ToggleConfig, waterui_toggle, label, toggle);

native_view!(
    ToggleConfig,
    waterui_toggle,
    waterui_view_force_as_toggle,
    waterui_view_toggle_id
);
