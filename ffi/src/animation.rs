use waterui::animation::Animation;

use crate::reactive::WuiWatcherMetadata;

use super::IntoFFI;

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
            .map(IntoFFI::into_ffi)
            .unwrap_or(WuiAnimation::None)
    }
}
