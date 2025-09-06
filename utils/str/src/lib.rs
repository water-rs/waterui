#![doc = include_str!("../README.md")]
#![allow(clippy::cast_possible_wrap)]
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
pub struct Str {
    /// Pointer to the string data.
    ptr: NonNull<()>,

    /// Length of the string in bytes.
    /// If len >= 0, it points to a static string.
    /// otherwise, it points to a Shared structure containing a reference-counted String,
    len: isize,
}

impl Drop for Str {
    /// Decrements the reference count for owned strings and frees the memory
    /// when the reference count reaches zero.
    ///
    /// For static strings, this is a no-op.
    fn drop(&mut self) {
        if let Ok(shared) = self.as_shared() {
            unsafe {
                if shared.is_unique() {
                    let ptr = self.ptr.cast::<Shared>().as_ptr();
                    let _ = Box::from_raw(ptr);
                } else {
                    shared.decrement_count();
                }
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
        if let Ok(shared) = self.as_shared() {
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
                len: s.len() as isize,
            }
        }
    }

    /// Creates a `Str` from raw parts.
    ///
    /// This function creates a `Str` directly from a raw pointer and length.
    /// The caller must ensure that the pointer and length represent valid string data.
    ///
    /// # Safety
    ///
    /// This function is unsafe because improper use may lead to memory unsafety. For it to be safe, the caller must ensure that:
    ///
    /// * `ptr` must be non-null and properly aligned
    /// * If `len >= 0`, `ptr` must point to valid UTF-8 string data of exactly `len` bytes
    /// * If `len < 0`, `ptr` must point to a valid `Shared` structure containing a reference-counted String
    /// * The memory pointed to by `ptr` must remain valid for the lifetime of the returned `Str`
    /// * The caller must not use the raw pointer after passing it to this function
    pub const unsafe fn from_raw(ptr: *mut (), len: isize) -> Self {
        Self {
            ptr: unsafe { NonNull::new_unchecked(ptr) },
            len,
        }
    }

    /// Consumes this `Str` and returns its raw parts.
    ///
    /// This function decomposes a `Str` into its internal representation:
    /// a raw pointer and a length value. The returned values can be used to
    /// reconstruct the `Str` later using [`from_raw`](Self::from_raw).
    ///
    /// # Returns
    ///
    /// A tuple containing:
    /// * `*mut ()` - A raw pointer to the string data
    /// * `isize` - The length information (positive for static strings, negative for owned strings)
    ///
    /// # Safety
    ///
    /// After calling this function, the caller is responsible for the memory management
    /// of the returned pointer. The pointer must eventually be passed back to
    /// [`from_raw`](Self::from_raw) to properly clean up the memory.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_str::Str;
    ///
    /// let s = Str::from("hello");
    /// let (ptr, len) = s.into_raw();
    ///
    /// // SAFETY: We immediately reconstruct the Str from the same raw parts
    /// let s2 = unsafe { Str::from_raw(ptr, len) };
    /// assert_eq!(s2, "hello");
    /// ```
    #[must_use]
    pub fn into_raw(self) -> (*mut (), isize) {
        let this = ManuallyDrop::new(self);
        (this.ptr.as_ptr(), this.len)
    }

    fn from_string(string: String) -> Self {
        let len = string.len();
        if len == 0 {
            // use static empty string
            return Self::new();
        }

        unsafe {
            Self {
                ptr: NonNull::new_unchecked(Box::into_raw(Box::new(Shared::new(string))))
                    .cast::<()>(),
                len: -(len as isize),
            }
        }
    }

    const fn is_shared(&self) -> bool {
        self.len < 0
    }

    const fn as_shared(&self) -> Result<&Shared, &'static str> {
        if !self.is_shared() {
            return Err(unsafe {
                core::str::from_utf8_unchecked(slice::from_raw_parts(
                    self.ptr.as_ptr().cast(),
                    self.len(),
                ))
            });
        }

        unsafe { Ok(self.ptr.cast::<Shared>().as_ref()) }
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
    pub const fn as_str(&self) -> &str {
        match self.as_shared() {
            Ok(shared) => unsafe { shared.as_str() },
            Err(str) => str,
        }
    }

    /// Returns the length of the string, in bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_str::Str;
    /// let s = Str::from("hello");
    /// assert_eq!(s.len(), 5);
    /// ```
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len.unsigned_abs()
    }

    /// Returns `true` if the string has a length of zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_str::Str;
    /// let s = Str::new();
    /// assert!(s.is_empty());
    /// let s2 = Str::from("not empty");
    /// assert!(!s2.is_empty());
    /// ```
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the reference count for owned strings, or `None` for static strings.
    ///
    /// This method is primarily useful for testing and debugging purposes.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_str::Str;
    ///
    /// let s1 = Str::from("static");
    /// assert_eq!(s1.reference_count(), None); // Static string
    ///
    /// let s2 = Str::from(String::from("owned"));
    /// assert_eq!(s2.reference_count(), Some(1)); // Owned string
    ///
    /// let s3 = s2.clone();
    /// assert_eq!(s2.reference_count(), Some(2)); // Reference count increased
    /// assert_eq!(s3.reference_count(), Some(2)); // Both point to same shared data
    /// ```
    #[must_use]
    pub const fn reference_count(&self) -> Option<usize> {
        match self.as_shared() {
            Ok(shared) => Some(shared.reference_count()),
            Err(_) => None,
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
    pub fn into_string(self) -> String {
        let this = ManuallyDrop::new(self);
        match this.as_shared() {
            Ok(shared) => unsafe {
                if shared.is_unique() {
                    let shared = Box::from_raw(this.ptr.cast::<Shared>().as_ptr());

                    shared.take()
                } else {
                    shared.decrement_count();
                    shared.as_str().to_string()
                }
            },
            Err(str) => str.to_string(),
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

    /// Applies a function to the owned string representation of this `Str`.
    ///
    /// This is an internal utility method used for operations that need to modify
    /// the string contents.
    fn handle(&mut self, f: impl FnOnce(&mut String)) {
        let mut string = take(self).into_string();
        f(&mut string);
        *self = Self::from(string);
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
        Self::from_string(value)
    }
}

impl From<Str> for String {
    fn from(value: Str) -> Self {
        value.into_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn test_static_string_creation() {
        let s = Str::from_static("hello");
        assert_eq!(s.as_str(), "hello");
        assert_eq!(s.len(), 5);
        assert!(!s.is_empty());
        assert_eq!(s.reference_count(), None);
    }

    #[test]
    fn test_owned_string_creation() {
        let s = Str::from(String::from("hello"));
        assert_eq!(s.as_str(), "hello");
        assert_eq!(s.len(), 5);
        assert!(!s.is_empty());
        assert_eq!(s.reference_count(), Some(1));
    }

    #[test]
    fn test_empty_string() {
        let s = Str::new();
        assert_eq!(s.as_str(), "");
        assert_eq!(s.len(), 0);
        assert!(s.is_empty());
        assert_eq!(s.reference_count(), None); // Empty string is static
    }

    #[test]
    fn test_static_string_clone() {
        let s1 = Str::from_static("hello");
        let s2 = s1.clone();

        assert_eq!(s1.as_str(), "hello");
        assert_eq!(s2.as_str(), "hello");
        assert_eq!(s1.reference_count(), None);
        assert_eq!(s2.reference_count(), None);
    }

    #[test]
    fn test_owned_string_clone() {
        let s1 = Str::from(String::from("hello"));
        assert_eq!(s1.reference_count(), Some(1));

        let s2 = s1.clone();
        assert_eq!(s1.reference_count(), Some(2));
        assert_eq!(s2.reference_count(), Some(2));

        assert_eq!(s1.as_str(), "hello");
        assert_eq!(s2.as_str(), "hello");
    }

    #[test]
    fn test_multiple_clones() {
        let s1 = Str::from(String::from("test"));
        let s2 = s1.clone();
        let s3 = s1.clone();
        let s4 = s2.clone();

        assert_eq!(s1.reference_count(), Some(4));
        assert_eq!(s2.reference_count(), Some(4));
        assert_eq!(s3.reference_count(), Some(4));
        assert_eq!(s4.reference_count(), Some(4));

        drop(s4);
        assert_eq!(s1.reference_count(), Some(3));
        assert_eq!(s2.reference_count(), Some(3));
        assert_eq!(s3.reference_count(), Some(3));

        drop(s3);
        drop(s2);
        assert_eq!(s1.reference_count(), Some(1));
    }

    #[test]
    fn test_reference_counting_drop() {
        let s1 = Str::from(String::from("hello"));
        assert_eq!(s1.reference_count(), Some(1));

        {
            let s2 = s1.clone();
            assert_eq!(s1.reference_count(), Some(2));
            assert_eq!(s2.reference_count(), Some(2));
        } // s2 is dropped here

        assert_eq!(s1.reference_count(), Some(1));
    }

    #[test]
    fn test_into_string_unique() {
        let s = Str::from(String::from("hello"));
        assert_eq!(s.reference_count(), Some(1));

        let string = s.into_string();
        assert_eq!(string, "hello");
    }

    #[test]
    fn test_into_string_shared() {
        let s1 = Str::from(String::from("hello"));
        let s2 = s1.clone();
        assert_eq!(s1.reference_count(), Some(2));

        let string = s1.into_string();
        assert_eq!(string, "hello");
        assert_eq!(s2.reference_count(), Some(1));
    }

    #[test]
    fn test_into_string_static() {
        let s = Str::from_static("hello");
        let string = s.into_string();
        assert_eq!(string, "hello");
    }

    #[test]
    fn test_from_utf8_valid() {
        let bytes = vec![104, 101, 108, 108, 111]; // "hello"
        let s = Str::from_utf8(bytes).unwrap();
        assert_eq!(s.as_str(), "hello");
        assert_eq!(s.reference_count(), Some(1));
    }

    #[test]
    fn test_from_utf8_invalid() {
        let bytes = vec![0xFF, 0xFF];
        assert!(Str::from_utf8(bytes).is_err());
    }

    #[test]
    fn test_from_utf8_unchecked() {
        let bytes = vec![104, 101, 108, 108, 111]; // "hello"
        let s = unsafe { Str::from_utf8_unchecked(bytes) };
        assert_eq!(s.as_str(), "hello");
        assert_eq!(s.reference_count(), Some(1));
    }

    #[test]
    fn test_append() {
        let mut s = Str::from("hello");
        s.append(" world");
        assert_eq!(s.as_str(), "hello world");
        assert_eq!(s.reference_count(), Some(1));
    }

    #[test]
    fn test_append_static_to_owned() {
        let mut s = Str::from_static("hello");
        assert_eq!(s.reference_count(), None);

        s.append(" world");
        assert_eq!(s.as_str(), "hello world");
        assert_eq!(s.reference_count(), Some(1));
    }

    #[test]
    fn test_as_bytes() {
        let s = Str::from("hello");
        assert_eq!(s.as_bytes(), b"hello");
    }

    #[test]
    fn test_deref() {
        let s = Str::from("hello");
        assert_eq!(&*s, "hello");
        assert_eq!(s.chars().count(), 5);
    }

    #[test]
    fn test_empty_string_from_string() {
        let s = Str::from(String::new());
        assert_eq!(s.as_str(), "");
        assert!(s.is_empty());
        assert_eq!(s.reference_count(), None); // Empty strings use static reference
    }

    // Memory safety tests designed for Miri
    #[test]
    fn test_memory_safety_clone_drop_cycles() {
        // Test multiple clone/drop cycles to ensure no memory leaks or double-frees
        for _ in 0..100 {
            let s1 = Str::from(String::from("test"));
            let s2 = s1.clone();
            let s3 = s2.clone();

            drop(s1);
            drop(s3);
            drop(s2);
        }
    }

    #[test]
    fn test_memory_safety_interleaved_operations() {
        let mut strings = vec![];

        // Create multiple strings with shared references
        for i in 0..10 {
            let mut content = String::from("string_");
            content.push_str(&(i.to_string()));
            let s = Str::from(content);
            strings.push(s.clone());
            strings.push(s);
        }

        // Randomly drop some strings
        for i in (0..strings.len()).step_by(3) {
            if i < strings.len() {
                strings.remove(i);
            }
        }

        // Verify remaining strings are still valid
        for s in &strings {
            assert!(!s.as_str().is_empty());
        }
    }

    #[test]
    fn test_memory_safety_reference_counting() {
        let original = Str::from(String::from("reference test"));
        #[allow(clippy::collection_is_never_read)]
        let mut clones = vec![];

        // Create many clones
        for _ in 0..50 {
            clones.push(original.clone());
        }

        assert_eq!(original.reference_count(), Some(51));

        // Drop half the clones
        clones.truncate(25);
        assert_eq!(original.reference_count(), Some(26));

        // Drop all clones
        clones.clear();
        assert_eq!(original.reference_count(), Some(1));
    }

    #[test]
    fn test_memory_safety_into_string_with_clones() {
        let s1 = Str::from(String::from("unique test"));
        let s2 = s1.clone();
        let s3 = s1.clone();

        assert_eq!(s1.reference_count(), Some(3));

        // Converting to string should not affect other references
        let string = s1.into_string();
        assert_eq!(string, "unique test");
        assert_eq!(s2.reference_count(), Some(2));
        assert_eq!(s3.reference_count(), Some(2));
    }

    #[test]
    fn test_memory_safety_unique_into_string() {
        // Test that unique references properly transfer ownership
        let s = Str::from(String::from("unique"));
        assert_eq!(s.reference_count(), Some(1));

        let string = s.into_string();
        assert_eq!(string, "unique");
        // s is consumed, can't check reference count
    }

    #[test]
    fn test_memory_safety_static_vs_owned() {
        let static_str = Str::from_static("static");
        let owned_str = Str::from(String::from("owned"));

        // Clone both types many times
        let mut static_clones = vec![];
        let mut owned_clones = vec![];

        for _ in 0..100 {
            static_clones.push(static_str.clone());
            owned_clones.push(owned_str.clone());
        }

        assert_eq!(static_str.reference_count(), None);
        assert_eq!(owned_str.reference_count(), Some(101));

        // Verify all clones work correctly
        for clone in &static_clones {
            assert_eq!(clone.as_str(), "static");
            assert_eq!(clone.reference_count(), None);
        }

        for clone in &owned_clones {
            assert_eq!(clone.as_str(), "owned");
            assert_eq!(clone.reference_count(), Some(101));
        }
    }

    #[test]
    fn test_memory_safety_mixed_operations() {
        let mut s = Str::from_static("hello");
        assert_eq!(s.reference_count(), None);

        // Convert to owned by appending
        s.append(" world");
        assert_eq!(s.reference_count(), Some(1));

        // Clone the owned string
        let s2 = s.clone();
        assert_eq!(s.reference_count(), Some(2));
        assert_eq!(s2.reference_count(), Some(2));

        // Convert back to string
        let string = s.into_string();
        assert_eq!(string, "hello world");
        assert_eq!(s2.reference_count(), Some(1));
    }

    #[test]
    fn test_memory_safety_zero_length_edge_cases() {
        // Test various ways to create empty strings
        let empty1 = Str::new();
        let empty2 = Str::from("");
        let empty3 = Str::from(String::new());
        let empty4 = Str::from_utf8(vec![]).unwrap();

        assert!(empty1.is_empty());
        assert!(empty2.is_empty());
        assert!(empty3.is_empty());
        assert!(empty4.is_empty());

        // All empty strings should be static references
        assert_eq!(empty1.reference_count(), None);
        assert_eq!(empty2.reference_count(), None);
        assert_eq!(empty3.reference_count(), None);
        assert_eq!(empty4.reference_count(), None);
    }

    #[test]
    fn test_memory_safety_large_strings() {
        // Test with larger strings to ensure proper memory handling
        let large_content = "x".repeat(10000);
        let s1 = Str::from(large_content.clone());
        assert_eq!(s1.reference_count(), Some(1));

        let s2 = s1.clone();
        assert_eq!(s1.reference_count(), Some(2));
        assert_eq!(s2.reference_count(), Some(2));

        assert_eq!(s1.len(), 10000);
        assert_eq!(s2.len(), 10000);
        assert_eq!(s1.as_str(), large_content);
        assert_eq!(s2.as_str(), large_content);
    }

    #[test]
    fn test_memory_safety_concurrent_like_pattern() {
        // Simulate concurrent-like access patterns (single-threaded but similar stress)
        let base = Str::from(String::from("base"));
        let mut handles = vec![];

        // Create many references
        for _ in 0..1000 {
            handles.push(base.clone());
        }

        assert_eq!(base.reference_count(), Some(1001));

        // Process in chunks, dropping some while keeping others
        for chunk in handles.chunks_mut(100) {
            for (i, handle) in chunk.iter().enumerate() {
                assert_eq!(handle.as_str(), "base");
                #[allow(clippy::manual_is_multiple_of)]
                if i % 2 == 0 {
                    // Mark for keeping (we'll drop the others)
                }
            }
        }

        // Keep only every 3rd element
        let mut i = 0;
        handles.retain(|_| {
            i += 1;
            i % 3 == 0
        });

        // Verify reference count updated correctly
        let expected_count = handles.len() + 1; // +1 for base
        assert_eq!(base.reference_count(), Some(expected_count));

        // Verify all remaining handles are valid
        for handle in &handles {
            assert_eq!(handle.as_str(), "base");
            assert_eq!(handle.reference_count(), Some(expected_count));
        }
    }

    #[test]
    fn test_memory_safety_drop_order_stress() {
        // Test various drop orders to ensure no use-after-free
        let s1 = Str::from(String::from("original"));
        let s2 = s1.clone();
        let s3 = s1.clone();
        let s4 = s2.clone();
        let s5 = s3.clone();

        assert_eq!(s1.reference_count(), Some(5));

        // Drop in different orders across multiple test runs
        {
            let temp1 = s1.clone();
            let temp2 = s2.clone();
            drop(temp2);
            drop(temp1);
            // temp1 and temp2 dropped first
        }

        assert_eq!(s1.reference_count(), Some(5));

        drop(s5); // Drop s5 first
        assert_eq!(s1.reference_count(), Some(4));

        drop(s2); // Drop s2 (middle)
        assert_eq!(s1.reference_count(), Some(3));

        drop(s1); // Drop original
        assert_eq!(s3.reference_count(), Some(2));

        drop(s4); // Drop s4
        assert_eq!(s3.reference_count(), Some(1));

        // s3 is the last one standing
        assert_eq!(s3.as_str(), "original");
    }

    #[test]
    fn test_memory_safety_ptr_stability() {
        // Ensure string content pointer remains stable across clones
        let s1 = Str::from(String::from("stable"));
        let ptr1 = s1.as_str().as_ptr();

        let s2 = s1.clone();
        let ptr2 = s2.as_str().as_ptr();

        // Clones should point to the same underlying data
        assert_eq!(ptr1, ptr2);

        let s3 = s2.clone();
        let ptr3 = s3.as_str().as_ptr();

        assert_eq!(ptr1, ptr3);
        assert_eq!(ptr2, ptr3);

        // Even after dropping some references, remaining should still be valid
        drop(s1);
        assert_eq!(s2.as_str().as_ptr(), ptr2);
        assert_eq!(s3.as_str().as_ptr(), ptr3);
    }

    #[test]
    fn test_memory_safety_alternating_clone_drop() {
        let original = Str::from(String::from("alternating"));
        let mut refs = vec![original];

        // Alternating pattern: clone, clone, drop, clone, drop, etc.
        for i in 0..100 {
            if i % 4 == 0 || i % 4 == 1 {
                // Clone phase
                let new_ref = refs[0].clone();
                refs.push(new_ref);
            } else if i % 4 == 2 && refs.len() > 1 {
                // Drop phase
                refs.pop();
            }

            // Verify all remaining references are valid
            for r in &refs {
                assert_eq!(r.as_str(), "alternating");
                assert_eq!(r.reference_count(), Some(refs.len()));
            }
        }
    }
}
