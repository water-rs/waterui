use waterui::app::App;
use waterui_controls::menu::resolve_menu_bar_items;

use crate::{
    IntoFFI, MenuItems, WuiEnv, array::WuiArray, reactive::WuiComputed, window::WuiWindow,
};

/// FFI-compatible representation of an application.
///
/// This struct is returned by value from `waterui_app()`.
/// Native code can read fields directly.
#[repr(C)]
pub struct WuiApp {
    /// Array of windows. The first window is the main window.
    pub windows: WuiArray<WuiWindow>,
    /// The application menu bar as resolved menu items.
    pub menu_bar: *mut WuiComputed<MenuItems>,
    /// The application environment containing injected services.
    /// Returned to native for use during rendering.
    pub env: *mut WuiEnv,
}

impl IntoFFI for App {
    type FFI = WuiApp;

    fn into_ffi(self) -> Self::FFI {
        let (windows, menu_bar, env) = self.into_parts();
        let menu_bar = resolve_menu_bar_items(&menu_bar, &env).into_ffi();
        WuiApp {
            windows: windows.into_ffi(),
            menu_bar,
            env: env.into_ffi(),
        }
    }
}
