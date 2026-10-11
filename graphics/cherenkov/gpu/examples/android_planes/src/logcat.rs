//! The `cherenkov-planes` heartbeat and engine-side messages through the
//! Android logger — stderr on other targets so host tests see them.

use std::ffi::{CStr, c_int};
#[cfg(target_os = "android")]
use std::ffi::{CString, c_char};

// ndk-sys declares the symbol without a `#[link]`; this crate supplies it.
#[cfg(target_os = "android")]
#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(priority: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

#[cfg(target_os = "android")]
const TAG: &CStr = c"cherenkov-planes";
const ENGINE_TAG: &CStr = c"cherenkov";

const INFO: c_int = 4;
const WARN: c_int = 5;
const ERROR: c_int = 6;

/// Writes `line` under the `cherenkov-planes` tag at INFO: the per-second
/// scenario heartbeat the verification reads.
#[cfg(target_os = "android")]
pub fn line(line: &str) {
    write(INFO, TAG, line);
}

/// Writes `line` under `cherenkov` at INFO.
pub fn info(line: &str) {
    write(INFO, ENGINE_TAG, line);
}

/// Writes `line` under `cherenkov` at WARN.
pub fn warn(line: &str) {
    write(WARN, ENGINE_TAG, line);
}

/// Writes `line` under `cherenkov` at ERROR.
pub fn error(line: &str) {
    write(ERROR, ENGINE_TAG, line);
}

#[cfg(target_os = "android")]
fn write(priority: c_int, tag: &CStr, line: &str) {
    let Ok(text) = CString::new(line) else {
        return;
    };
    unsafe {
        __android_log_write(priority, tag.as_ptr(), text.as_ptr());
    }
}

#[cfg(not(target_os = "android"))]
fn write(_priority: c_int, tag: &CStr, line: &str) {
    eprintln!("{}: {line}", tag.to_string_lossy());
}
