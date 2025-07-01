#![doc = include_str!("../README.md")]
#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;

mod impls;
mod shared;
use alloc::{
    borrow::Cow,
    boxed::Box,
    string::{FromUtf8Error, String, ToString},
    vec::Vec,
};
use shared::Shared;

use core::{
    borrow::Borrow,
    mem::{ManuallyDrop, take},
    ops::Deref,
    ptr::NonNull,
    slice,
};

/// A string type that can be either a static reference or a ref-counted owned string.
///
/// `Str` combines the benefits of both `&'static str` and `String` with efficient
/// cloning and passing, automatically using the most appropriate representation
/// based on the source.
#[derive(Debug)]
#[repr(C)]
pub struct Str {
    /// Pointer to the string data.
    ///
    /// If the pointer value is less than [`usize::MAX`] / 2, it points to a static string.
    /// Otherwise, it points to a Shared structure containing a reference-counted String,
    /// offset by [`usize::MAX`] / 2.
    ptr: NonNull<()>,

    /// Length of the string in bytes.
    len: usize,
}

impl Drop for Str {
    /// Decrements the reference count for owned strings and frees the memory
    /// when the reference count reaches zero.
    ///
    /// For static strings, this is a no-op.
    fn drop(&mut self) {
        if let Ok(shared) = self.try_as_mut_shared() {
            unsafe {
                shared.decrement_count();
            }

            if shared.count() == 0 {
                take(shared);
            }
        }
    }
}

impl Clone for Str {
    /// Creates a clone of the string.
    ///
    /// For static strings, this is a simple pointer copy.
    /// For owned strings, this increments the reference count.
    fn clone(&self) -> Self {
        if let Err(shared) = self.try_as_static() {
            unsafe {
                shared.increment_count();
            }
        }

        Self {
            ptr: self.ptr,
            len: self.len,
        }
    }
}

impl Deref for Str {
    type Target = str;

    /// Provides access to the underlying string slice.
    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl Borrow<str> for Str {
    /// Allows borrowing a `Str` as a string slice.
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for Str {
    /// Converts `Str` to a string slice reference.
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<[u8]> for Str {
    /// Converts `Str` to a byte slice reference.
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl Default for Str {
    /// Creates a new empty `Str`.
    fn default() -> Self {
        Self::new()
    }
}

impl From<Cow<'static, str>> for Str {
    /// Creates a `Str` from a `Cow<'static, str>`.
    ///
    /// This will borrow from static strings and own dynamic strings.
    fn from(value: Cow<'static, str>) -> Self {
        match value {
            Cow::Borrowed(s) => s.into(),
            Cow::Owned(s) => s.into(),
        }
    }
}

/// Implementations available when the `std` library is available.
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
        /// Converts `Str` to an OS string slice reference.
        fn as_ref(&self) -> &OsStr {
            self.deref().as_ref()
        }
    }

    impl AsRef<Path> for Str {
        /// Converts `Str` to a path reference.
        fn as_ref(&self) -> &Path {
            self.deref().as_ref()
        }
    }

    impl TryFrom<OsString> for Str {
        type Error = FromUtf8Error;

        /// Attempts to create a `Str` from an `OsString`.
        ///
        /// This will fail if the `OsString` contains invalid UTF-8 data.
        fn try_from(value: OsString) -> Result<Self, Self::Error> {
            Self::from_utf8(value.into_encoded_bytes())
        }
    }

    impl ToSocketAddrs for Str {
        type Iter = IntoIter<SocketAddr>;

        /// Converts a string to a socket address.
        fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
            self.deref().to_socket_addrs()
        }
    }
}

impl Str {
    /// Creates a new empty `Str`.
    ///
    /// This returns a static empty string reference.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_str::Str;
    ///
    /// let s = Str::new();
    /// assert_eq!(s, "");
    /// assert_eq!(s.reference_count(), None); // Static reference
    /// ```
    #[must_use]
    pub const fn new() -> Self {
        Self::from_static("")
    }

    /// Consumes the `Str` and returns its raw parts.
    ///
    /// The returned pointer and length can be used to create a new `Str` with
    /// `from_raw_parts`. This is primarily intended for cases where you need
    /// to pass ownership of the string through FFI boundaries.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_str::Str;
    ///
    /// let s = Str::from("hello");
    /// let (ptr, len) = s.into_raw_parts();
    ///
    /// // Recreate the Str from raw parts
    /// let s = unsafe { Str::from_raw_parts(ptr, len) };
    /// assert_eq!(s, "hello");
    /// ```
    #[must_use]
    pub fn into_raw_parts(self) -> (*const (), usize) {
        let this = ManuallyDrop::new(self);
        (this.ptr.as_ptr(), this.len)
    }

    /// # Safety
    ///
    /// This function is unsafe because it creates a `Str` from a raw pointer
    /// and length without validating if the memory contains valid UTF-8 or
    /// if the pointer points to valid allocated memory. The caller must
    /// ensure the pointer is valid for reads of `len` bytes and contains
    /// valid UTF-8 data.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_str::Str;
    ///
    /// let s1 = Str::from("hello");
    /// let (ptr, len) = s1.into_raw_parts();
    ///
    /// let s2 = unsafe { Str::from_raw_parts(ptr, len) };
    /// assert_eq!(s2, "hello");
    /// ```
    #[must_use]
    pub const unsafe fn from_raw_parts(ptr: *const (), len: usize) -> Self {
        unsafe {
            Self {
                ptr: NonNull::new_unchecked(ptr.cast_mut()),
                len,
            }
        }
    }

    /// Creates a `Str` from a vector of bytes.
    ///
    /// This function will attempt to convert the vector to a UTF-8 string and
    /// wrap it in a `Str`. If the vector does not contain valid UTF-8, an error
    /// is returned.
    ///
    /// # Errors
    ///
    /// Returns an error if the provided byte vector does not contain valid UTF-8 data.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_str::Str;
    ///
    /// let bytes = vec![104, 101, 108, 108, 111]; // "hello" in UTF-8
    /// let s = Str::from_utf8(bytes).unwrap();
    /// assert_eq!(s, "hello");
    /// assert_eq!(s.reference_count(), Some(1)); // Owned string
    ///
    /// // Invalid UTF-8 sequence
    /// let invalid = vec![0xFF, 0xFF];
    /// assert!(Str::from_utf8(invalid).is_err());
    /// ```
    pub fn from_utf8(bytes: Vec<u8>) -> Result<Self, FromUtf8Error> {
        String::from_utf8(bytes).map(Self::from)
    }

    /// Creates a `Str` from a static string slice.
    ///
    /// This method allows creating a `Str` from a string with a static lifetime,
    /// which will be stored as a pointer to the static data without any allocation.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_str::Str;
    ///
    /// let s = Str::from_static("hello");
    /// assert_eq!(s, "hello");
    /// assert_eq!(s.reference_count(), None); // Static reference
    /// ```
    #[must_use]
    pub const fn from_static(s: &'static str) -> Self {
        unsafe {
            Self {
                ptr: NonNull::new_unchecked(s.as_ptr().cast_mut().cast::<()>()),
                len: s.len(),
            }
        }
    }

    /// # Safety
    ///
    /// This function is unsafe because it does not check that the bytes passed
    /// to it are valid UTF-8. If this constraint is violated, it may cause
    /// memory unsafety issues with future users of the `Str`.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_str::Str;
    ///
    /// // SAFETY: We know these bytes form valid UTF-8
    /// let bytes = vec![104, 101, 108, 108, 111]; // "hello" in UTF-8
    /// let s = unsafe { Str::from_utf8_unchecked(bytes) };
    /// assert_eq!(s, "hello");
    /// ```
    #[must_use]
    pub unsafe fn from_utf8_unchecked(bytes: Vec<u8>) -> Self {
        unsafe { Self::from(String::from_utf8_unchecked(bytes)) }
    }

    /// Returns `true` if this `Str` is a static reference.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_str::Str;
    ///
    /// let static_str = Str::from("static");
    /// let owned_str = Str::from(String::from("owned"));
    ///
    /// assert!(static_str.is_static());
    /// assert!(!owned_str.is_static());
    /// ```
    fn is_static(&self) -> bool {
        (self.ptr.as_ptr() as usize) < usize::MAX / 2
    }

    /// Applies a function to the owned string representation of this `Str`.
    ///
    /// This is an internal utility method used for operations that need to modify
    /// the string contents.
    fn handle(&mut self, f: impl FnOnce(&mut String)) {
        let mut string = take(self).into_string();
        f(&mut string);
        *self = Self::from(string);
    }

    /// # Safety
    ///
    /// This function assumes that `self` is a static string reference, and will
    /// cause undefined behavior if called on a `Str` containing an owned string.
    #[must_use]
    const unsafe fn as_static_unchecked(&self) -> &'static str {
        unsafe {
            let slice = slice::from_raw_parts(self.ptr.as_ptr() as *const u8, self.len);
            core::str::from_utf8_unchecked(slice)
        }
    }

    /// # Safety
    ///
    /// This function assumes that `self` is an owned string, and will cause
    /// undefined behavior if called on a `Str` containing a static string reference.
    const unsafe fn as_shared_unchecked(&self) -> &Shared {
        unsafe {
            let ptr = self.ptr.as_ptr().byte_sub(usize::MAX / 2);
            &*(ptr as *const Shared)
        }
    }

    /// # Safety
    ///
    /// This function assumes that `self` is an owned string, and will cause
    /// undefined behavior if called on a `Str` containing a static string reference.
    const unsafe fn as_shared_mut_ptr_unchecked(&mut self) -> *mut Shared {
        unsafe {
            let ptr = self.ptr.as_ptr().byte_sub(usize::MAX / 2);
            ptr.cast::<Shared>()
        }
    }

    /// # Safety
    ///
    /// This function assumes that `self` is an owned string, and will cause
    /// undefined behavior if called on a `Str` containing a static string reference.
    unsafe fn as_shared_mut_unchecked(&mut self) -> &mut Shared {
        unsafe { &mut *self.as_shared_mut_ptr_unchecked() }
    }

    /// Attempts to get a static string reference.
    ///
    /// Returns `Ok` with a static string if this `Str` contains a static reference,
    /// or `Err` with a reference to the shared data structure if it contains an owned string.
    fn try_as_static(&self) -> Result<&'static str, &Shared> {
        unsafe {
            if self.is_static() {
                Ok(self.as_static_unchecked())
            } else {
                Err(self.as_shared_unchecked())
            }
        }
    }

    /// Attempts to get a mutable reference to the shared data.
    ///
    /// Returns `Ok` with a mutable reference to the shared data if this `Str` contains
    /// an owned string, or `Err` with a static string reference if it contains a static string.
    fn try_as_mut_shared(&mut self) -> Result<&mut Shared, &'static str> {
        unsafe {
            if self.is_static() {
                Err(self.as_static_unchecked())
            } else {
                Ok(self.as_shared_mut_unchecked())
            }
        }
    }

    /// Returns the reference count of the string if it's an owned string.
    ///
    /// Returns `None` if the string is a static reference.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_str::Str;
    ///
    /// let static_str = Str::from("static");
    /// let owned_str = Str::from(String::from("owned"));
    ///
    /// assert_eq!(static_str.reference_count(), None);
    /// assert_eq!(owned_str.reference_count(), Some(1));
    ///
    /// let clone = owned_str.clone();
    /// assert_eq!(owned_str.reference_count(), Some(2));
    /// assert_eq!(clone.reference_count(), Some(2));
    /// ```
    #[must_use]
    pub fn reference_count(&self) -> Option<usize> {
        if let Err(shared) = self.try_as_static() {
            Some(shared.count())
        } else {
            None
        }
    }

    /// Converts this `Str` into a `String`.
    ///
    /// For static strings, this will allocate a new string and copy the contents.
    /// For owned strings, this will attempt to take ownership of the string if the reference
    /// count is 1, or create a new copy otherwise.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_str::Str;
    ///
    /// let s1 = Str::from("static");
    /// let s1_string = s1.into_string();
    /// assert_eq!(s1_string, "static");
    ///
    /// let s2 = Str::from(String::from("owned"));
    /// let s2_string = s2.into_string();
    /// assert_eq!(s2_string, "owned");
    /// ```
    #[must_use]
    pub fn into_string(mut self) -> String {
        let len = self.len;
        unsafe {
            match self.try_as_mut_shared() {
                Ok(shared) => shared
                    .take(len)
                    .unwrap_or_else(|| shared.as_str(len).to_string()),
                Err(s) => s.to_string(),
            }
        }
    }

    /// Returns a string slice of this `Str`.
    ///
    /// This method works for both static and owned strings.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_str::Str;
    ///
    /// let s1 = Str::from("hello");
    /// assert_eq!(s1.as_str(), "hello");
    ///
    /// let s2 = Str::from(String::from("world"));
    /// assert_eq!(s2.as_str(), "world");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.try_as_static()
            .unwrap_or_else(|shared| shared.as_str(self.len))
    }

    /// Appends a string to this `Str`.
    ///
    /// This method will convert the `Str` to an owned string if it's a static reference.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_str::Str;
    ///
    /// let mut s = Str::from("hello");
    /// s.append(" world");
    /// assert_eq!(s, "hello world");
    /// ```
    pub fn append(&mut self, s: impl AsRef<str>) {
        let mut string = take(self).into_string();
        string.push_str(s.as_ref());
        *self = Self::from(string);
    }
}
impl From<&'static str> for Str {
    /// Creates a `Str` from a static string slice.
    ///
    /// This stores a reference to the original string without any allocation.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_str::Str;
    ///
    /// let s = Str::from("hello");
    /// assert_eq!(s, "hello");
    /// assert_eq!(s.reference_count(), None); // Static reference
    /// ```
    fn from(value: &'static str) -> Self {
        Self::from_static(value)
    }
}

impl From<String> for Str {
    /// Creates a `Str` from an owned `String`.
    ///
    /// This will store the string in a reference-counted container.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_str::Str;
    ///
    /// let s = Str::from(String::from("hello"));
    /// assert_eq!(s, "hello");
    /// assert_eq!(s.reference_count(), Some(1)); // Owned string
    /// ```
    fn from(value: String) -> Self {
        let len = value.len();
        let ptr = Box::into_raw(Box::new(Shared::new(value))).cast::<()>();
        let ptr = ptr.wrapping_byte_add(usize::MAX / 2);
        unsafe {
            Self {
                ptr: NonNull::new_unchecked(ptr),
                len,
            }
        }
    }
}

impl From<Str> for String {
    fn from(value: Str) -> Self {
        value.into_string()
    }
}
