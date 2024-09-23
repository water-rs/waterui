#![no_std]
extern crate alloc;

mod impls;
mod shared;
use alloc::{
    boxed::Box,
    string::{FromUtf8Error, String, ToString},
    vec::Vec,
};
use shared::Shared;

use waterui_reactive::impl_constant;

impl_constant!(Str);

use core::{
    borrow::Borrow,
    mem::{take, ManuallyDrop},
    ops::Deref,
    ptr::{read, NonNull},
    slice,
};

#[derive(Debug)]
pub struct Str {
    ptr: NonNull<()>,
    len: usize,
}

impl Drop for Str {
    fn drop(&mut self) {
        if let Err(shared) = self.try_as_static() {
            unsafe { shared.decrement_count() }
        }
    }
}

impl Clone for Str {
    fn clone(&self) -> Self {
        if let Err(shared) = self.try_as_static() {
            unsafe { shared.increment_count() }
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
        self.as_str()
    }
}

impl Borrow<str> for Str {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for Str {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<[u8]> for Str {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl Default for Str {
    fn default() -> Self {
        Self::new()
    }
}

mod std_on {
    use alloc::{string::FromUtf8Error, vec::IntoIter};

    use crate::Str;

    extern crate std;

    use core::{net::SocketAddr, ops::Deref};
    use std::{
        ffi::{OsStr, OsString},
        io,
        net::ToSocketAddrs,
        path::Path,
    };

    impl AsRef<OsStr> for Str {
        fn as_ref(&self) -> &OsStr {
            self.deref().as_ref()
        }
    }

    impl AsRef<Path> for Str {
        fn as_ref(&self) -> &Path {
            self.deref().as_ref()
        }
    }

    impl TryFrom<OsString> for Str {
        type Error = FromUtf8Error;
        fn try_from(value: OsString) -> Result<Self, Self::Error> {
            Str::from_utf8(value.into_encoded_bytes())
        }
    }

    impl ToSocketAddrs for Str {
        type Iter = IntoIter<SocketAddr>;
        fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
            self.deref().to_socket_addrs()
        }
    }
}

impl Str {
    pub const fn new() -> Self {
        Self::from_static("")
    }

    pub fn from_utf8(bytes: Vec<u8>) -> Result<Self, FromUtf8Error> {
        String::from_utf8(bytes).map(Self::from)
    }

    pub const fn from_static(s: &'static str) -> Self {
        unsafe {
            Self {
                ptr: NonNull::new_unchecked(s.as_ptr() as *mut ()),
                len: s.len(),
            }
        }
    }

    /// # Safety
    ///
    /// This function is unsafe because it does not check that the bytes passed
    /// to it are valid UTF-8. If this constraint is violated, it may cause
    /// memory unsafety issues with future users of the `Str`.
    pub unsafe fn from_utf8_unchecked(bytes: Vec<u8>) -> Self {
        Self::from(String::from_utf8_unchecked(bytes))
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

    fn try_as_static(&self) -> Result<&'static str, &Shared> {
        unsafe {
            if self.is_static() {
                Ok(self.as_static_unchecked())
            } else {
                Err(self.as_shared_unchecked())
            }
        }
    }

    pub fn into_string(self) -> String {
        let this = ManuallyDrop::new(self);
        unsafe {
            if this.is_static() {
                this.as_static_unchecked().to_string()
            } else {
                let ptr = this.ptr.as_ptr().byte_sub(usize::MAX / 2) as *mut Shared;
                let shared = read(ptr);
                shared.take(this.len())
            }
        }
    }

    pub fn as_str(&self) -> &str {
        self.try_as_static()
            .unwrap_or_else(|shared| shared.as_str(self.len))
    }
}
impl From<&'static str> for Str {
    fn from(value: &'static str) -> Self {
        Self::from_static(value)
    }
}

impl From<String> for Str {
    fn from(value: String) -> Self {
        let len = value.len();
        let ptr = Box::into_raw(Box::new(Shared::new(value))) as *mut ();
        let ptr = ptr.wrapping_byte_add(usize::MAX / 2);
        unsafe {
            Self {
                ptr: NonNull::new_unchecked(ptr),
                len,
            }
        }
    }
}
