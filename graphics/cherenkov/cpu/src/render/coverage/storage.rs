//! Disjoint typed tables in the compiler's one transient allocation.

use std::mem::MaybeUninit;
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;

#[derive(Debug)]
pub(super) struct Table<T> {
    data: NonNull<T>,
    len: usize,
    capacity: usize,
}

impl<T> Default for Table<T> {
    fn default() -> Self {
        Self {
            data: NonNull::dangling(),
            len: 0,
            capacity: 0,
        }
    }
}

// SAFETY: tables have one owner, and the backing allocation moves with
// that owner. No reference survives a compiler operation or crosses workers.
unsafe impl<T: Send> Send for Table<T> {}

impl<T> Table<T> {
    #[cfg(not(feature = "bounded-bins"))]
    pub(super) const fn clear(&mut self) {
        self.len = 0;
    }

    /// # Safety
    /// The supplied allocation outlives this table and is not resized while
    /// it is used. T has alignment at most u64, and its portion is disjoint
    /// from all other tables. Elements must not require destruction: the
    /// backing allocation is released without running their destructors.
    /// Only the initialized prefix is exposed.
    #[expect(
        clippy::mut_mut,
        reason = "advances the caller's remaining allocation slice"
    )]
    pub(super) unsafe fn bind(words: &mut &mut [MaybeUninit<u64>], capacity: usize) -> Self {
        let count = (capacity * size_of::<T>()).div_ceil(size_of::<u64>());
        let (head, tail) = std::mem::take(words).split_at_mut(count);
        *words = tail;
        Self {
            data: NonNull::new(head.as_mut_ptr().cast()).expect("slice pointer is non-null"),
            len: 0,
            capacity,
        }
    }

    pub(super) const fn push(&mut self, value: T) {
        assert!(self.len < self.capacity, "coverage table capacity exceeded");
        // SAFETY: this slot is in the bound allocation and becomes initialized
        // before len is advanced; no reference to unused storage is created.
        unsafe {
            self.data.as_ptr().add(self.len).write(value);
        }
        self.len += 1;
    }

    pub(super) const fn resize(&mut self, len: usize, value: T)
    where
        T: Copy,
    {
        while self.len < len {
            self.push(value);
        }
        self.len = len;
    }
}

impl<T> Deref for Table<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        // SAFETY: the table's initialized prefix belongs to its live owner.
        unsafe { std::slice::from_raw_parts(self.data.as_ptr(), self.len) }
    }
}

impl<T> DerefMut for Table<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        // SAFETY: tables are disjoint and this borrow is exclusive.
        unsafe { std::slice::from_raw_parts_mut(self.data.as_ptr(), self.len) }
    }
}
