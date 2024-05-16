use alloc::boxed::Box;

use crate::IntoRust;

#[repr(C)]
pub struct waterui_fn {
    data: *mut (),
    call: unsafe extern "C" fn(*const ()),
    drop: unsafe extern "C" fn(*mut ()),
}

impl IntoRust for waterui_fn {
    type Rust = Box<dyn Fn()>;
    unsafe fn into_rust(self) -> Self::Rust {
        Box::new(move || self.call())
    }
}

impl waterui_fn {
    pub unsafe fn new(
        data: *mut (),
        call: unsafe extern "C" fn(*const ()),
        drop: unsafe extern "C" fn(*mut ()),
    ) -> Self {
        Self { data, call, drop }
    }
    pub fn call(&self) {
        unsafe { (self.call)(self.data) }
    }
}

impl Drop for waterui_fn {
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
