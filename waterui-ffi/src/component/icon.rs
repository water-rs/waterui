use crate::computed::{waterui_computed_double, waterui_computed_str};
use ::waterui_icon::IconConfig;

#[repr(C)]
pub struct waterui_icon {
    name: *mut waterui_computed_str,
    size: *mut waterui_computed_double,
}

into_ffi!(IconConfig, waterui_icon, name, size);

native_view!(
    IconConfig,
    waterui_icon,
    waterui_view_force_as_icon,
    waterui_view_icon_id
);
