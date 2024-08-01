use waterui::app::App;

#[repr(C)]
pub struct waterui_app {
    env: *mut waterui_env,
    bridge: *mut waterui_bridge,
}

impl IntoFFI for App {
    type FFI = waterui_app;
    fn into_ffi(self) -> Self::FFI {
        waterui_app {
            env: self._env.into_ffi(),
            bridge: self._bridge.into_ffi(),
        }
    }
}

// Must be called on Rust thread
#[no_mangle]
unsafe extern "C" fn waterui_launch_app(app: waterui_app) {
    (*app.env).executor().run();
}

use waterui::app::Bridge;

use crate::waterui_env;
use crate::IntoFFI;

ffi_type!(waterui_bridge, Bridge, waterui_drop_bridge);

#[repr(C)]
pub struct waterui_bridge_closure {
    data: *mut (),
    call: unsafe extern "C" fn(*mut ()),
}

unsafe impl Send for waterui_bridge_closure {}
unsafe impl Sync for waterui_bridge_closure {}

impl waterui_bridge_closure {
    pub unsafe fn new(data: *mut (), call: unsafe extern "C" fn(*mut ())) -> Self {
        Self { data, call }
    }

    pub fn call(self) {
        unsafe { (self.call)(self.data) }
    }
}
#[no_mangle]
unsafe extern "C" fn waterui_bridge_send(
    bridge: *const waterui_bridge,
    task: waterui_bridge_closure,
) {
    (*bridge).send_blocking(|| task.call()).unwrap();
}
