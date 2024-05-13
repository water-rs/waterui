use alloc::boxed::Box;

use crate::IntoFFI;

ffi_type!(waterui_fn, Box<dyn Fn()>, waterui_drop_fn);

struct FFIFn {
    data: *mut (),
    call: unsafe extern "C" fn(*const ()),
    drop: unsafe extern "C" fn(*mut ()),
}

impl FFIFn {
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

impl Drop for FFIFn {
    fn drop(&mut self) {
        unsafe { (self.drop)(self.data) }
    }
}

pub unsafe extern "C" fn waterui_new_fn(
    data: *mut (),
    call: unsafe extern "C" fn(*const ()),
    drop: unsafe extern "C" fn(*mut ()),
) -> *mut waterui_fn {
    let f = FFIFn::new(data, call, drop);
    let boxed: Box<dyn Fn()> = Box::new(move || f.call());
    boxed.into_ffi()
}
