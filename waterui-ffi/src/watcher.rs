use waterui_reactive::watcher::{self};

ffi_type!(
    waterui_watcher_metadata,
    watcher::Metadata,
    waterui_drop_watcher_metadata
);

#[repr(C)]
pub struct waterui_watcher<T> {
    data: *mut (),
    call: unsafe extern "C" fn(*const (), T, *const waterui_watcher_metadata),
    drop: unsafe extern "C" fn(*mut ()),
}

impl<T: 'static> waterui_watcher<T> {
    pub unsafe fn new(
        data: *mut (),
        call: unsafe extern "C" fn(*const (), T, *const waterui_watcher_metadata),
        drop: unsafe extern "C" fn(*mut ()),
    ) -> Self {
        Self { data, call, drop }
    }
    pub fn call(&self, value: T, metadata: watcher::Metadata) {
        unsafe {
            (self.call)(
                self.data,
                value,
                (&metadata) as *const watcher::Metadata as *const waterui_watcher_metadata,
            )
        }
    }
}

impl<T> Drop for waterui_watcher<T> {
    fn drop(&mut self) {
        unsafe { (self.drop)(self.data) }
    }
}
