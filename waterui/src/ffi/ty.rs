use core::{
    marker::{PhantomData, PhantomPinned},
    mem::transmute,
    ops::Deref,
    ptr::slice_from_raw_parts,
};

use alloc::{boxed::Box, string::String};

use super::app::App;

#[repr(C)]
#[derive(Debug)]
pub struct Utf8Data {
    head: *mut u8,
    len: usize,
}

impl Deref for Utf8Data {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        unsafe { core::str::from_utf8_unchecked(&*slice_from_raw_parts(self.head, self.len)) }
    }
}

impl_array!(Data, u8, u8);

impl Utf8Data {
    pub fn into_data(self) -> Data {
        Data {
            head: self.head,
            len: self.len,
        }
    }
}

impl From<Utf8Data> for String {
    fn from(val: Utf8Data) -> Self {
        unsafe { String::from_utf8_unchecked(val.into_data().into()) }
    }
}

impl From<String> for Utf8Data {
    fn from(value: String) -> Self {
        let data = Data::from(value.into_bytes());
        Self {
            head: data.head,
            len: data.len,
        }
    }
}

#[repr(C)]
pub struct Closure {
    data: *mut (),
    call: unsafe extern "C" fn(*const ()),
    free: unsafe extern "C" fn(*mut ()),
}

unsafe impl Send for Closure {}
unsafe impl Sync for Closure {}

impl Closure {
    pub fn call(&self) {
        unsafe { (self.call)(self.data) }
    }
}

impl Drop for Closure {
    fn drop(&mut self) {
        unsafe { (self.free)(self.data) }
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

impl AppClosure {
    pub fn call(&self, app: crate::App) {
        unsafe { (self.call)(self.data, app.into()) }
    }
}

impl Drop for AppClosure {
    fn drop(&mut self) {
        unsafe { (self.free)(self.data) }
    }
}

#[repr(C)]
pub struct TypeId {
    inner: [u64; 2],
    _marker: PhantomData<(*const (), PhantomPinned)>,
}

#[allow(clippy::missing_transmute_annotations)]
impl From<core::any::TypeId> for TypeId {
    fn from(value: core::any::TypeId) -> Self {
        unsafe {
            Self {
                inner: transmute(value),
                _marker: PhantomData,
            }
        }
    }
}

impl From<TypeId> for core::any::TypeId {
    fn from(value: TypeId) -> Self {
        unsafe { transmute(value.inner) }
    }
}

ffi_opaque!(Box<dyn Fn(&crate::Environment)>, Action, 2);

ffi_opaque!(crate::Environment, Environment, 1);

// WARNING: You must call this function on Rust thread.
#[no_mangle]
unsafe extern "C" fn waterui_clone_env(env: *const Environment) -> Environment {
    (*env).clone().into()
}

#[no_mangle]
unsafe extern "C" fn waterui_drop_env(env: Environment) {
    let _ = env;
}

#[no_mangle]
unsafe extern "C" fn waterui_free_action(action: Action) {
    let _ = action.into_ty();
}

#[no_mangle]
unsafe extern "C" fn waterui_call_action(action: *const Action, environment: *const Environment) {
    (*action)((*environment).deref());
}
