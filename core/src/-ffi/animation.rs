use crate::animation::Animation;

use super::{IntoFFI, watcher::WuiWatcherMetadata};

#[repr(C)]
pub enum WuiAnimation {
    Default,
    None,
}

impl IntoFFI for Animation {
    type FFI = WuiAnimation;
    fn into_ffi(self) -> Self::FFI {
        WuiAnimation::Default
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn waterui_get_animation(metadata: *const WuiWatcherMetadata) -> WuiAnimation {
    unsafe {
        (*metadata)
            .try_get::<Animation>()
            .map(IntoFFI::ffi_struct)
            .unwrap_or(WuiAnimation::None)
    }
}
