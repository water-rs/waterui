#[repr(C)]
pub struct waterui_closure {
    data: *mut (),
    call: unsafe extern "C" fn(*const ()),
    free: unsafe extern "C" fn(*mut ()),
}

unsafe impl Send for waterui_closure {}
unsafe impl Sync for waterui_closure {}

impl Drop for waterui_closure {
    fn drop(&mut self) {
        unsafe { (self.free)(self.data) }
    }
}

impl waterui_closure {
    pub fn call(&self) {
        unsafe { (self.call)(self.data) }
    }
}
