use waterui::{Str, window::Window};
use waterui_layout::Rect;

use crate::{
    IntoFFI, WuiAnyView,
    reactive::{WuiBinding, WuiComputed},
};

#[repr(C)]
pub struct WuiWindow {
    title: *mut WuiComputed<Str>,
    closable: bool,
    frame: *mut WuiBinding<Rect>,
    content: *mut WuiAnyView,
}

impl IntoFFI for Window {
    type FFI = WuiWindow;

    fn into_ffi(self) -> Self::FFI {
        WuiWindow {
            title: self.title.into_ffi(),
            closable: self.closable,
            frame: self.frame.into_ffi(),
            content: self.content.into_ffi(),
        }
    }
}
