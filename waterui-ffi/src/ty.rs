use core::{
    marker::{PhantomData, PhantomPinned},
    mem::transmute,
    ops::Deref,
    ptr,
};

use alloc::{borrow::Cow, boxed::Box, string::String};

use crate::{IntoFFI, IntoRust};

ffi_safe!(u8, isize, bool);

impl_array!(Data, u8, u8);
ffi_opaque!(AnyView, waterui_view::AnyView, 2, waterui_drop_anyview);

ffi_view!(
    waterui_view::AnyView,
    AnyView,
    waterui_view_force_as_any,
    waterui_view_any_id
);

ffi_opaque!(Environment, waterui_view::Environment, 4, waterui_drop_env);
ffi_opaque!(
    Action,
    Box<dyn Fn(&waterui_view::Environment)>,
    2,
    waterui_drop_action
);

#[no_mangle]
unsafe extern "C" fn waterui_call_action(action: *const Action, env: *const Environment) {
    (*action)(&*env);
}

impl_array!(Views, waterui_view::AnyView, AnyView);

ffi_clone!(waterui_clone_env, Environment);

#[repr(C)]
pub struct Closure {
    data: *mut (),
    call: unsafe extern "C" fn(*const ()),
    free: unsafe extern "C" fn(*mut ()),
}

unsafe impl Send for Closure {}
unsafe impl Sync for Closure {}

impl Drop for Closure {
    fn drop(&mut self) {
        unsafe { (self.free)(self.data) }
    }
}

impl Closure {
    pub fn call(&self) {
        unsafe { (self.call)(self.data) }
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct Utf8Data {
    head: *mut u8,
    len: usize,
}

impl Deref for Utf8Data {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        unsafe {
            let slice = &*ptr::slice_from_raw_parts(self.head, self.len);
            core::str::from_utf8_unchecked(slice)
        }
    }
}

impl IntoFFI for String {
    type FFI = Utf8Data;
    fn into_ffi(mut self) -> Self::FFI {
        let len = self.len();
        let head = self.as_mut_ptr();
        core::mem::forget(self);
        Utf8Data { head, len }
    }
}

impl IntoFFI for Cow<'static, str> {
    type FFI = Utf8Data;
    fn into_ffi(self) -> Self::FFI {
        self.into_owned().into_ffi()
    }
}

impl IntoRust for Utf8Data {
    type Rust = String;
    fn into_rust(self) -> Self::Rust {
        unsafe {
            Box::from_raw(core::str::from_utf8_unchecked_mut(
                &mut *ptr::slice_from_raw_parts_mut(self.head, self.len),
            ))
            .into_string()
        }
    }
}

#[repr(C)]
pub struct TypeId {
    inner: [u64; 2],
    _marker: PhantomData<(*const (), PhantomPinned)>,
}

impl IntoFFI for core::any::TypeId {
    type FFI = TypeId;
    fn into_ffi(self) -> Self::FFI {
        unsafe {
            TypeId {
                inner: transmute::<core::any::TypeId, [u64; 2]>(self),
                _marker: PhantomData,
            }
        }
    }
}

impl IntoRust for TypeId {
    type Rust = core::any::TypeId;
    fn into_rust(self) -> Self::Rust {
        unsafe { transmute(self.inner) }
    }
}
