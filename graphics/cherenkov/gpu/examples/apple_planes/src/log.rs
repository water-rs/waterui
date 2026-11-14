//! The `dev.cherenkov.planes` heartbeat and engine-side messages through
//! `os_log` on iOS.
//!
//! On iOS the variadic `os_log` C entry points are not linkable from a
//! plain arm64 binary (the SDK's `libSystem` re-exports them for arm64e
//! only), so the Swift host owns the `os.Logger` — subsystem
//! `dev.cherenkov.planes` — and this module calls it through the
//! `cherenkov_planes_os_log` `@_cdecl` bridge.
//!
//! Other targets only exist so host tests compile: the lines become
//! `tracing` events at the matching level — with the harness's own
//! subscriber installed they are filtered out (the `Forward` layer
//! passes `cherenkov` targets only), so they carry no output of their
//! own there.

#[cfg(target_os = "ios")]
use std::ffi::{CString, c_char};

/// Priority handed to the bridge: the heartbeat's default level or the
/// error level.
#[cfg(target_os = "ios")]
#[repr(u8)]
enum Level {
    Default = 0,
    Error = 1,
}

#[cfg(target_os = "ios")]
unsafe extern "C" {
    /// The host's `os.Logger` bridge, defined in Swift.
    fn cherenkov_planes_os_log(kind: u8, message: *const c_char);
}

/// Writes `line` at the default level under the harness subsystem: the
/// per-second scenario heartbeat the verification reads.
#[cfg(target_os = "ios")]
pub fn line(line: &str) {
    write(Level::Default, line);
}

/// Writes `line` at the error level.
#[cfg(target_os = "ios")]
pub fn error(line: &str) {
    write(Level::Error, line);
}

#[cfg(target_os = "ios")]
fn write(kind: Level, line: &str) {
    let Ok(text) = CString::new(line) else {
        return;
    };
    // SAFETY: the host defines the symbol; the C string outlives the
    // call.
    unsafe {
        cherenkov_planes_os_log(kind as u8, text.as_ptr());
    }
}

/// Writes `line` at the default level.
#[cfg(not(target_os = "ios"))]
pub fn line(line: &str) {
    tracing::info!("{line}");
}

/// Writes `line` at the error level.
#[cfg(not(target_os = "ios"))]
pub fn error(line: &str) {
    tracing::error!("{line}");
}
