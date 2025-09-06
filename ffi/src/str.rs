//! FFI bindings for the Str type.
//!
//! This module provides a C API for the Str type, allowing it to be used from C code.
//! The API is designed to be memory-safe while still providing the full functionality
//! of the Str type.

use core::ffi::{c_char, c_int, c_uint};
use core::ptr::null_mut;

use crate::{IntoFFI, IntoRust, impl_drop};
use alloc::ffi::CString;
use alloc::string::String;

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
pub unsafe extern "C" fn waterui_str_from_cstr(c_str: *const c_char) -> *mut WuiStr {
    let c_str = unsafe { core::ffi::CStr::from_ptr(c_str) };
    Str::from(String::from(c_str.to_str().unwrap())).into_ffi()
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
pub unsafe extern "C" fn waterui_str_concat(str1: *const Str, str2: *const Str) -> *mut WuiStr {
    let s1 = unsafe { &*str1 };
    let s2 = unsafe { &*str2 };
    (s1.clone() + s2.as_str()).into_ffi()
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
pub unsafe extern "C" fn waterui_str_from_bytes(bytes: *const c_char, len: c_uint) -> *mut WuiStr {
    let slice = unsafe { core::slice::from_raw_parts(bytes as *const u8, len as usize) };
    unsafe { Str::from(core::str::from_utf8_unchecked(slice)).into_ffi() }
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
