use waterui_ffi::{AnyView, Environment, IntoFFI, IntoRust, TypeId};
use waterui_view::View;

use crate::ViewExt;

#[repr(C)]
pub struct App {
    content: AnyView,
    env: Environment,
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
unsafe extern "C" fn waterui_view_id(view: *const AnyView) -> TypeId {
    (*view).type_id().into_ffi()
}

#[no_mangle]
extern "C" fn waterui_call_view(view: AnyView, env: crate::ffi::Environment) -> AnyView {
    view.into_rust().body(env.into_rust()).anyview().into_ffi()
}

#[no_mangle]
extern "C" fn waterui_view_empty_id() -> TypeId {
    core::any::TypeId::of::<()>().into_ffi()
}
