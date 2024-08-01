#![no_std]
extern crate alloc;
use alloc::{
    boxed::Box,
    string::{String, ToString},
};

use core::{
    borrow::Borrow,
    cell::Cell,
    fmt::Display,
    hash::Hash,
    mem::{take, ManuallyDrop},
    ops::{Add, AddAssign, Deref},
    ptr::NonNull,
    slice,
};

#[derive(Debug)]
pub struct Str {
    ptr: NonNull<()>,
    len: usize,
}

struct Shared {
    ptr: NonNull<u8>,
    capacity: usize,
    count: Cell<usize>,
}

impl Drop for Str {
    fn drop(&mut self) {
        if let Ok(shared) = self.try_as_shared() {
            if shared.count.get() == 1 {
                unsafe {
                    let _ = String::from_raw_parts(shared.ptr.as_ptr(), self.len, shared.capacity);
                }
            } else {
                shared.count.set(shared.count.get() - 1);
            }
        }
    }
}

impl Clone for Str {
    fn clone(&self) -> Self {
        if let Ok(shared) = self.try_as_shared() {
            shared.count.set(shared.count.get() + 1);
        }

        Self {
            ptr: self.ptr,
            len: self.len,
        }
    }
}

impl Deref for Str {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        match self.try_as_shared() {
            Ok(shared) => unsafe {
                core::str::from_utf8_unchecked(slice::from_raw_parts(shared.ptr.as_ptr(), self.len))
            },
            Err(s) => s,
        }
    }
}

impl Borrow<str> for Str {
    fn borrow(&self) -> &str {
        self.deref()
    }
}

impl AsRef<[u8]> for Str {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl Default for Str {
    fn default() -> Self {
        Self::empty()
    }
}

impl Str {
    pub const fn empty() -> Self {
        Self::from_static("")
    }

    pub const fn from_static(s: &'static str) -> Self {
        unsafe {
            Self {
                ptr: NonNull::new_unchecked(s.as_ptr() as *mut ()),
                len: s.len(),
            }
        }
    }

    fn is_static(&self) -> bool {
        (self.ptr.as_ptr() as usize) < usize::MAX / 2
    }

    fn handle(&mut self, f: impl FnOnce(&mut String)) {
        let mut string = take(self).into_string();
        f(&mut string);
        *self = Str::from(string);
    }

    unsafe fn as_static_unchecked(&self) -> &'static str {
        let slice = slice::from_raw_parts(self.ptr.as_ptr() as *const u8, self.len);
        core::str::from_utf8_unchecked(slice)
    }

    unsafe fn as_shared_unchecked(&self) -> &Shared {
        let ptr = self.ptr.as_ptr().byte_sub(usize::MAX / 2);
        &*(ptr as *const Shared)
    }

    fn try_as_shared(&self) -> Result<&Shared, &'static str> {
        unsafe {
            if self.is_static() {
                Err(self.as_static_unchecked())
            } else {
                Ok(self.as_shared_unchecked())
            }
        }
    }

    pub fn into_string(self) -> String {
        let this = ManuallyDrop::new(self);
        if let Ok(shared) = this.try_as_shared() {
            if shared.count.get() == 1 {
                unsafe {
                    return String::from_raw_parts(shared.ptr.as_ptr(), this.len, shared.capacity);
                }
            }
        }

        this.deref().to_string()
    }
}

impl AsRef<str> for Str {
    fn as_ref(&self) -> &str {
        self.deref()
    }
}

impl Hash for Str {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.deref().hash(state)
    }
}

impl PartialEq for Str {
    fn eq(&self, other: &Self) -> bool {
        self.deref().eq(other.deref())
    }
}

impl Eq for Str {}

impl PartialOrd for Str {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Str {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.deref().cmp(other.deref())
    }
}

impl Display for Str {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.deref().fmt(f)
    }
}

impl<'a> Extend<&'a str> for Str {
    fn extend<T: IntoIterator<Item = &'a str>>(&mut self, iter: T) {
        self.handle(move |string| {
            string.extend(iter);
        });
    }
}

impl Extend<String> for Str {
    fn extend<T: IntoIterator<Item = String>>(&mut self, iter: T) {
        self.handle(move |string| {
            string.extend(iter);
        });
    }
}

impl Extend<Str> for Str {
    fn extend<T: IntoIterator<Item = Str>>(&mut self, iter: T) {
        self.handle(move |string| {
            for s in iter.into_iter() {
                string.push_str(&s);
            }
        });
    }
}

impl From<&'static str> for Str {
    fn from(value: &'static str) -> Self {
        Self::from_static(value)
    }
}

impl<T> Add<T> for Str
where
    T: AsRef<str>,
{
    type Output = Self;
    fn add(self, rhs: T) -> Self::Output {
        let rhs = rhs.as_ref();
        (self.into_string() + rhs).into()
    }
}

impl<T> AddAssign<T> for Str
where
    T: AsRef<str>,
{
    fn add_assign(&mut self, rhs: T) {
        let rhs = rhs.as_ref();

        let string = take(self).into_string();
        *self = (string + rhs).into();
    }
}

impl From<String> for Str {
    fn from(value: String) -> Self {
        let len = value.len();
        let ptr = Box::into_raw(Box::new(Shared::from(value))) as *mut ();
        let ptr = ptr.wrapping_byte_add(usize::MAX / 2);
        unsafe {
            Self {
                ptr: NonNull::new_unchecked(ptr),
                len,
            }
        }
    }
}

impl From<String> for Shared {
    fn from(value: String) -> Self {
        let mut value = ManuallyDrop::new(value);
        unsafe {
            Self {
                ptr: NonNull::new_unchecked(value.as_mut_ptr()),
                capacity: value.capacity(),
                count: Cell::new(1),
            }
        }
    }
}
