use core::{
    marker::{PhantomData, PhantomPinned},
    mem::transmute,
};

use alloc::{boxed::Box, string::String};

#[repr(C)]
pub struct Utf8Data {
    head: *mut u8,
    len: usize,
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

ffi_opaque!(Box<dyn Fn()+>, Action, 2);

#[no_mangle]
unsafe extern "C" fn waterui_free_action(action: Action) {
    let _ = action.into_ty();
}

#[no_mangle]
unsafe extern "C" fn waterui_call_action(action: *const Action) {
    (*action)();
}
