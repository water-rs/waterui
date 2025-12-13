use waterui::app::App;

use crate::{IntoFFI, WuiEnv, array::WuiArray, window::WuiWindow};

/// FFI-compatible representation of an application.
///
/// This struct is returned by value from `waterui_app()`.
/// Native code can read fields directly.
#[repr(C)]
pub struct WuiApp {
    /// Array of windows. The first window is the main window.
    pub windows: WuiArray<WuiWindow>,
    /// The application environment containing injected services.
    /// Returned to native for use during rendering.
    pub env: *mut WuiEnv,
}

impl IntoFFI for App {
    type FFI = WuiApp;

    fn into_ffi(self) -> Self::FFI {
        WuiApp {
            windows: self.windows.into_ffi(),
            env: self.env.into_ffi(),
        }
    }
}
