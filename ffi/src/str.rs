//! FFI bindings for the Str type.
//!
//! This module provides a C API for the Str type, allowing it to be used from C code.
//! The API is designed to be memory-safe while still providing the full functionality
//! of the Str type.

use crate::IntoFFI;

use waterui::Str;

ffi_type!(WuiStr, Str, waterui_str_drop);

/// Creates a new empty Str instance.
///
/// # Returns
///
/// A new empty Str instance.
#[unsafe(no_mangle)]
pub extern "C" fn waterui_str_new() -> *mut WuiStr {
    Str::new().into_ffi()
}

/// Creates a clone of the given Str instance.
///
/// # Parameters
///
/// * `str` - A pointer to a valid Str instance
///
/// # Returns
///
/// A new Str instance that is a clone of the input Str.
///
/// # Safety
///
/// The caller must ensure that `str` is a valid pointer to a Str instance.
/// If `str` is null or invalid, undefined behavior will occur.
#[unsafe(no_mangle)]
unsafe extern "C" fn waterui_str_clone(str: *const Str) -> *mut WuiStr {
    unsafe { (*str).clone().into_ffi() }
}

/// Returns the length of the Str in bytes.
///
/// # Parameters
///
/// * `str` - A pointer to a valid Str instance
///
/// # Returns
///
/// The length of the string in bytes.
///
/// # Safety
///
/// The caller must ensure that `str` is a valid pointer to a Str instance.
/// If `str` is null or points to invalid memory, undefined behavior will occur.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_str_len(str: *const Str) -> usize {
    let s = unsafe { &*str };
    s.len()
}

/// Concatenates two Str instances.
///
/// # Parameters
///
/// * `str1` - A pointer to the first valid Str instance
/// * `str2` - A pointer to the second valid Str instance
///
/// # Returns
///
/// A new Str instance that is the concatenation of str1 and str2.
///
/// # Safety
///
/// The caller must ensure that both `str1` and `str2` are valid pointers to Str instances.
/// If either pointer is null or points to invalid memory, undefined behavior will occur.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_str_concat(str1: *const Str, str2: *const Str) -> *mut WuiStr {
    let s1 = unsafe { &*str1 };
    let s2 = unsafe { &*str2 };
    (s1.clone() + s2.as_str()).into_ffi()
}

/// Creates a Str by copying a byte array.
///
/// # Parameters
///
/// * `bytes` - A pointer to a byte array
/// * `len` - The length of the byte array
///
/// # Returns
///
/// A new Str instance containing the bytes interpreted as UTF-8.
///
/// # Safety
///
/// The caller must ensure that:
/// * `bytes` is a valid pointer to a byte array of at least `len` bytes
/// * The bytes must form a valid UTF-8 string
/// * The memory referenced by `bytes` remains valid for the duration of this call
///
/// This function uses `from_utf8_unchecked` internally, so providing invalid UTF-8 will result
/// in undefined behavior.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_str_from_bytes(bytes: *const u8, len: usize) -> *mut WuiStr {
    let slice = unsafe { core::slice::from_raw_parts(bytes, len) };
    // Warning: This slice is temporarily borrowed from foreign code.
    let vec = slice.to_vec();
    unsafe { Str::from_utf8_unchecked(vec).into_ffi() }
}

/// Gets a pointer to the raw UTF-8 bytes of a Str.
///
/// This function returns a pointer to the actual string data, regardless of whether
/// the string is static or heap-allocated. The returned pointer is valid as long as
/// the Str instance exists.
///
/// # Parameters
///
/// * `str` - A pointer to a valid Str instance
///
/// # Returns
///
/// A pointer to the UTF-8 bytes, or null if the input is null.
///
/// # Safety
///
/// The caller must ensure that:
/// * `str` is a valid pointer to a Str instance
/// * The returned pointer is not used after the Str is dropped
/// * The data pointed to is not modified
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_str_as_ptr(str: *const Str) -> *const u8 {
    let s = unsafe { &*str };
    s.as_str().as_ptr()
}
