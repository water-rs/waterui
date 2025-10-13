use waterui::prelude::lazy::Lazy;

use crate::{IntoFFI, views::WuiAnyViews};

#[repr(C)]
pub struct WuiLazy {
    contents: *mut WuiAnyViews,
}

impl IntoFFI for Lazy {
    type FFI = WuiLazy;

    fn into_ffi(self) -> Self::FFI {
        WuiLazy {
            contents: self.into_inner().into_ffi(),
        }
    }
}
ffi_view!(Lazy, WuiLazy, waterui_lazy_id, waterui_force_as_lazy);
