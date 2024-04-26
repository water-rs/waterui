pub use waterui_ffi::*;

use waterui_view::View;

use crate::ViewExt;

#[repr(C)]
pub struct App {
    content: *mut waterui_anyview,
    env: *mut waterui_env,
}

impl IntoFFI for crate::App {
    type FFI = App;
    fn into_ffi(self) -> Self::FFI {
        App {
            content: self._content.into_ffi(),
            env: self._env.into_ffi(),
        }
    }
}

#[repr(C)]
pub struct AppClosure {
    data: *mut (),
    call: unsafe extern "C" fn(*const (), App),
    free: unsafe extern "C" fn(*mut ()),
}

unsafe impl Send for AppClosure {}
unsafe impl Sync for AppClosure {}

impl Drop for AppClosure {
    fn drop(&mut self) {
        unsafe { (self.free)(self.data) }
    }
}

impl AppClosure {
    pub fn call(&self, app: crate::App) {
        unsafe { (self.call)(self.data, app.into_ffi()) }
    }
}

#[no_mangle]
pub unsafe extern "C" fn waterui_view_id(view: *const waterui_anyview) -> waterui_type_id {
    (*view).type_id().into_ffi()
}

#[no_mangle]
pub unsafe extern "C" fn waterui_call_view(
    view: *mut waterui_anyview,
    env: *mut waterui_env,
) -> *mut waterui_anyview {
    view.into_rust().body(env.into_rust()).anyview().into_ffi()
}

#[no_mangle]
pub extern "C" fn waterui_view_empty_id() -> waterui_type_id {
    core::any::TypeId::of::<()>().into_ffi()
}
