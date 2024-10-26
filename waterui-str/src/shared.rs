use core::{cell::Cell, mem::ManuallyDrop, ptr::slice_from_raw_parts};

use alloc::string::String;

#[repr(C)]
pub struct Shared {
    head: *mut u8,
    capacity: usize,
    count: Cell<usize>,
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

    pub fn as_str(&self, len: usize) -> &str {
        unsafe { core::str::from_utf8_unchecked(&*slice_from_raw_parts(self.head, len)) }
    }
    pub unsafe fn try_take(&self, len: usize) -> Option<String> {
        if self.count.get() == 1 {
            Some(String::from_raw_parts(self.head, len, self.capacity))
        } else {
            None
        }
    }

    pub unsafe fn increment_count(&self) {
        self.count.set(self.count.get().checked_add(1).unwrap());
    }
    pub unsafe fn decrement_count(&self) {
        if self.count.get() == 1 {
            unsafe {
                let _ = String::from_raw_parts(self.head, 0, self.capacity);
            }
        } else {
            self.count.set(self.count.get() - 1);
        }
    }
}
