use waterui::modifier::Padding;

use crate::IntoFFI;

#[repr(C)]
pub struct waterui_padding {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

impl IntoFFI for Padding {
    type FFI = waterui_padding;
    fn into_ffi(self) -> Self::FFI {
        waterui_padding {
            top: self.0.top,
            right: self.0.right,
            bottom: self.0.bottom,
            left: self.0.left,
        }
    }
}

ffi_metadata!(
    Padding,
    waterui_padding,
    waterui_metadata_force_as_padding,
    waterui_metadata_padding_id
);
