use alloc::boxed::Box;

use crate::IntoRust;

#[repr(C)]
pub struct waterui_fn<T> {
    data: *mut (),
    call: unsafe extern "C" fn(*const (), T),
    drop: unsafe extern "C" fn(*mut ()),
}

// TODO : Drop
impl<T: 'static> IntoRust for waterui_fn<T> {
    type Rust = Box<dyn Fn(T)>;
    unsafe fn into_rust(self) -> Self::Rust {
        Box::new(move |v| (self.call)(self.data, v))
    }
}

impl<T: 'static> waterui_fn<T> {
    pub unsafe fn new(
        data: *mut (),
        call: unsafe extern "C" fn(*const (), T),
        drop: unsafe extern "C" fn(*mut ()),
    ) -> Self {
        Self { data, call, drop }
    }
    pub fn call(&self, value: T) {
        unsafe { (self.call)(self.data, value) }
    }
}

impl<T> Drop for waterui_fn<T> {
    fn drop(&mut self) {
        unsafe { (self.drop)(self.data) }
    }
}

#[repr(C)]
pub struct waterui_fnonce {
    data: *mut (),
    call: unsafe extern "C" fn(*mut ()),
}

impl IntoRust for waterui_fnonce {
    type Rust = Box<dyn FnOnce()>;
    unsafe fn into_rust(self) -> Self::Rust {
        Box::new(move || self.call())
    }
}

impl waterui_fnonce {
    pub unsafe fn new(data: *mut (), call: unsafe extern "C" fn(*mut ())) -> Self {
        Self { data, call }
    }
    pub fn call(self) {
        unsafe { (self.call)(self.data) }
    }
}
