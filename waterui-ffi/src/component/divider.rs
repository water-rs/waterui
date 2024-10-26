use waterui::component::divder::Divider;

use crate::IntoFFI;

use super::waterui_nothing;

#[repr(C)]
#[derive(Debug, Default)]
pub struct waterui_divider(waterui_nothing);

impl IntoFFI for Divider {
    type FFI = waterui_divider;
    fn into_ffi(self) -> Self::FFI {
        waterui_divider::default()
    }
}

ffi_view!(
    Divider,
    waterui_divider,
    waterui_view_force_as_divider,
    waterui_view_divider_id
);
