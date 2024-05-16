ffi_type!(waterui_bridge, ::waterui_bridge::Bridge);

#[repr(C)]
pub struct waterui_bridge_closure {
    data: *mut (),
    call: unsafe extern "C" fn(*mut ()),
}

unsafe impl Send for waterui_bridge_closure {}
unsafe impl Sync for waterui_bridge_closure {}

impl waterui_bridge_closure {
    pub unsafe fn new(data: *mut (), call: unsafe extern "C" fn(*mut ())) -> Self {
        Self { data, call }
    }
    pub fn call(self) {
        unsafe { (self.call)(self.data) }
    }
}

#[no_mangle]
unsafe extern "C" fn waterui_bridge_send(bridge: *const waterui_bridge, f: waterui_bridge_closure) {
    (*bridge).send_blocking(move || f.call()).unwrap();
}
