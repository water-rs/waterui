use core::{
    cell::Cell,
    mem::{ManuallyDrop, take},
    ptr::slice_from_raw_parts,
};

use alloc::string::String;

#[repr(C)]
#[derive(Debug)]
pub struct Shared {
    head: *mut u8,
    capacity: usize,
    count: Cell<usize>,
}

impl Default for Shared {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl Shared {
    pub fn new(s: String) -> Self {
        let mut s = ManuallyDrop::new(s);
        let head = s.as_mut_ptr();
        let capacity = s.capacity();
        Self {
            head,
            capacity,
            count: Cell::new(1),
        }
    }

    pub fn count(&self) -> usize {
        self.count.get()
    }

    pub fn as_str(&self, len: usize) -> &str {
        unsafe { core::str::from_utf8_unchecked(&*slice_from_raw_parts(self.head, len)) }
    }

    pub fn is_unique(&self) -> bool {
        self.count() == 1
    }

    pub unsafe fn take(&mut self, len: usize) -> Option<String> {
        if self.is_unique() {
            let this = ManuallyDrop::new(take(self));
            unsafe { Some(String::from_raw_parts(this.head, len, this.capacity)) }
        } else {
            None
        }
    }

    pub unsafe fn increment_count(&self) {
        self.count.set(self.count().checked_add(1).unwrap());
    }

    pub unsafe fn decrement_count(&self) {
        self.count.set(self.count().checked_add_signed(-1).unwrap());
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        unsafe {
            let _ = String::from_raw_parts(self.head, 0, self.capacity);
        }
    }
}
