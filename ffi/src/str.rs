//! FFI bindings for the Str type.
//!
//! This module provides a C API for the Str type, allowing it to be used from C code.
//! The API is designed to be memory-safe while still providing the full functionality
//! of the Str type.

use core::ffi::{c_char, c_int, c_uint};
use core::ptr::null_mut;

use crate::{IntoFFI, ffi_safe, impl_drop};
use alloc::ffi::CString;
use alloc::string::String;

use waterui::Str;

ffi_safe!(Str);

/// Creates a new empty Str instance.
///
/// # Returns
///
/// A new empty Str instance.
#[unsafe(no_mangle)]
pub extern "C" fn waterui_str_new() -> Str {
    Str::new().into_ffi()
}

/// Creates a new Str instance from a C string.
///
/// # Parameters
///
/// * `c_str` - A null-terminated C string pointer
///
/// # Returns
///
/// A new Str instance containing the content of the C string.
///
/// # Safety
///
/// The caller must ensure that:
/// * `c_str` is a valid pointer to a null-terminated C string
/// * `c_str` points to a valid UTF-8 encoded string
/// * The memory referenced by `c_str` remains valid for the duration of this call
///
/// Undefined behavior (UB) will occur if any of these conditions are violated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_str_from_cstr(c_str: *const c_char) -> Str {
    let c_str = unsafe { core::ffi::CStr::from_ptr(c_str) };
    Str::from(String::from(c_str.to_str().unwrap()))
}

impl_drop!(Str, waterui_str_drop);

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
unsafe extern "C" fn waterui_str_clone(str: *const Str) -> Str {
    unsafe { (*str).clone() }
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
pub unsafe extern "C" fn waterui_str_len(str: *const Str) -> c_uint {
    let s = unsafe { &*str };
    s.len() as c_uint
}

/// Checks if the Str is empty.
///
/// # Parameters
///
/// * `str` - A pointer to a valid Str instance
///
/// # Returns
///
/// 1 if the string is empty, 0 otherwise.
///
/// # Safety
///
/// The caller must ensure that `str` is a valid pointer to a Str instance.
/// If `str` is null or points to invalid memory, undefined behavior will occur.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_str_is_empty(str: *const Str) -> c_int {
    let s = unsafe { &*str };
    if s.is_empty() { 1 } else { 0 }
}

/// Converts a Str to a C string.
///
/// # Parameters
///
/// * `str` - A pointer to a valid Str instance
///
/// # Returns
///
/// A pointer to a new null-terminated C string or NULL if conversion fails.
/// The caller is responsible for freeing this memory using the appropriate C function.
///
/// # Safety
///
/// The caller must ensure that:
/// * `str` is a valid pointer to a Str instance
/// * The returned C string must be freed by the caller to avoid memory leaks
///
/// If `str` is null or points to invalid memory, undefined behavior will occur.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_str_to_cstr(str: *const Str) -> *mut c_char {
    let s = unsafe { &*str };
    match CString::new(s.as_str()) {
        Ok(c_str) => c_str.into_raw(),
        Err(_) => null_mut(),
    }
}

/// Appends a C string to the end of a Str.
///
/// # Parameters
///
/// * `str` - A pointer to a valid Str instance that will be modified
/// * `c_str` - A null-terminated C string to append
///
/// # Safety
///
/// The caller must ensure that:
/// * `str` is a valid pointer to a Str instance
/// * `c_str` is a valid pointer to a null-terminated C string
/// * `c_str` points to a valid UTF-8 encoded string
/// * The memory referenced by both pointers remains valid for the duration of this call
///
/// Undefined behavior will occur if any of these conditions are violated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_str_append(str: *mut Str, c_str: *const c_char) {
    let c_str = unsafe { core::ffi::CStr::from_ptr(c_str) };
    unsafe { (*str).append(c_str.to_str().unwrap()) }
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
pub unsafe extern "C" fn waterui_str_concat(str1: *const Str, str2: *const Str) -> Str {
    let s1 = unsafe { &*str1 };
    let s2 = unsafe { &*str2 };
    s1.clone() + s2.as_str()
}

/// Compares two Str instances.
///
/// # Parameters
///
/// * `str1` - A pointer to the first valid Str instance
/// * `str2` - A pointer to the second valid Str instance
///
/// # Returns
///
/// A negative value if str1 < str2, 0 if str1 == str2, and a positive value if str1 > str2.
///
/// # Safety
///
/// The caller must ensure that both `str1` and `str2` are valid pointers to Str instances.
/// If either pointer is null or points to invalid memory, undefined behavior will occur.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_str_compare(str1: *const Str, str2: *const Str) -> c_int {
    let s1 = unsafe { &*str1 };
    let s2 = unsafe { &*str2 };
    s1.cmp(s2) as c_int
}

/// Gets the reference count of a Str instance.
///
/// # Parameters
///
/// * `str` - A pointer to a valid Str instance
///
/// # Returns
///
/// The reference count of the Str, or -1 if the pointer is null, or 0 if the Str doesn't
/// support reference counting.
///
/// # Safety
///
/// The caller must ensure that `str` is either null or a valid pointer to a Str instance.
/// If `str` is invalid but not null, undefined behavior will occur.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_str_ref_count(str: *const Str) -> c_int {
    if str.is_null() {
        return -1;
    }

    let s = unsafe { &*str };
    match s.reference_count() {
        Some(count) => count as c_int,
        None => 0,
    }
}

/// Creates a substring from a Str instance.
///
/// # Parameters
///
/// * `str` - A pointer to a valid Str instance
/// * `start` - The starting byte index (inclusive)
/// * `end` - The ending byte index (exclusive)
///
/// # Returns
///
/// A new Str instance containing the substring.
///
/// # Safety
///
/// The caller must ensure that:
/// * `str` is a valid pointer to a Str instance
/// * `start` and `end` form a valid range within the string's length
/// * The range forms a valid UTF-8 boundary
///
/// This function uses `get_unchecked` internally, so providing an invalid range will result
/// in undefined behavior.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_str_substring(str: *const Str, start: c_uint, end: c_uint) -> Str {
    let s = unsafe { &*str };
    unsafe { s.get_unchecked(start as usize..end as usize).into() }
}

/// Checks if a Str contains a substring.
///
/// # Parameters
///
/// * `str` - A pointer to a valid Str instance to search in
/// * `substring` - A pointer to a valid Str instance to search for
///
/// # Returns
///
/// 1 if the substring is found, 0 otherwise.
///
/// # Safety
///
/// The caller must ensure that both `str` and `substring` are valid pointers to Str instances.
/// If either pointer is null or points to invalid memory, undefined behavior will occur.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_str_contains(str: *const Str, substring: *const Str) -> c_int {
    let s = unsafe { &*str };
    let sub = unsafe { &*substring };

    if s.contains(sub.as_str()) { 1 } else { 0 }
}

/// Creates a Str from a byte array.
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
pub unsafe extern "C" fn waterui_str_from_bytes(bytes: *const c_char, len: c_uint) -> Str {
    let slice = unsafe { core::slice::from_raw_parts(bytes as *const u8, len as usize) };
    unsafe { core::str::from_utf8_unchecked(slice).into() }
}
