use waterui::{App, View, ViewExt};

use crate::{waterui_anyview, waterui_env, waterui_type_id, IntoFFI, IntoRust};

#[repr(C)]
pub struct waterui_app {
    content: *mut waterui_anyview,
    env: *mut waterui_env,
}

impl IntoFFI for App {
    type FFI = waterui_app;
    fn into_ffi(self) -> Self::FFI {
        waterui_app {
            content: self._content.into_ffi(),
            env: self._env.into_ffi(),
        }
    }
}

#[repr(C)]
pub struct waterui_app_closure {
    data: *mut (),
    call: unsafe extern "C" fn(*const (), waterui_app),
    free: unsafe extern "C" fn(*mut ()),
}

unsafe impl Send for waterui_app_closure {}
unsafe impl Sync for waterui_app_closure {}

impl Drop for waterui_app_closure {
    fn drop(&mut self) {
        unsafe { (self.free)(self.data) }
    }
}

impl waterui_app_closure {
    pub fn call(&self, app: App) {
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
    env: *const waterui_env,
) -> *mut waterui_anyview {
    view.into_rust().body(&*env).anyview().into_ffi()
}

#[no_mangle]
pub extern "C" fn waterui_view_empty_id() -> waterui_type_id {
    core::any::TypeId::of::<()>().into_ffi()
}
