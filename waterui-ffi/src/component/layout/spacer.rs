use waterui::component::layout::spacer::Spacer;

use crate::{component::waterui_nothing, IntoFFI};

#[repr(C)]
#[derive(Debug, Default)]
pub struct waterui_spacer(waterui_nothing);

impl IntoFFI for Spacer {
    type FFI = waterui_spacer;
    fn into_ffi(self) -> Self::FFI {
        waterui_spacer::default()
    }
}

ffi_view!(
    Spacer,
    waterui_spacer,
    waterui_view_force_as_spacer,
    waterui_view_spacer_id
);
