use waterui::animation::Animation;

use crate::{watcher::waterui_watcher_metadata, IntoFFI};

#[repr(C)]
pub enum waterui_animation {
    DEFAULT,
    NONE,
}

impl IntoFFI for Animation {
    type FFI = waterui_animation;
    fn into_ffi(self) -> Self::FFI {
        waterui_animation::DEFAULT
    }
}

#[no_mangle]
unsafe extern "C" fn waterui_get_animation(
    metadata: *const waterui_watcher_metadata,
) -> waterui_animation {
    (*metadata)
        .get::<Animation>()
        .cloned()
        .map(IntoFFI::into_ffi)
        .unwrap_or(waterui_animation::NONE)
}
