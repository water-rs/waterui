use core::cell::Cell;

use alloc::string::String;

#[derive(Debug)]
pub struct Shared {
    data: String,
    count: Cell<usize>,
}

impl Default for Shared {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl Shared {
    pub const fn new(data: String) -> Self {
        Self {
            data,
            count: Cell::new(1),
        }
    }

    // Internal: do not expose reference count publicly.
    const fn reference_count(&self) -> usize {
        self.count.get()
    }

    pub const unsafe fn as_str(&self) -> &str {
        self.data.as_str()
    }

    pub const fn is_unique(&self) -> bool {
        self.reference_count() == 1
    }

    pub unsafe fn take(self) -> String {
        self.data
    }

    pub unsafe fn increment_count(&self) {
        self.count.set(self.count.get() + 1);
    }

    pub unsafe fn decrement_count(&self) {
        let new_count = self.count.get().saturating_sub(1);
        self.count.set(new_count);
    }
}
