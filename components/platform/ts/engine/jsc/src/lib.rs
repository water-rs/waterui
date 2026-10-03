//! The `JavaScriptCore` implementation of [`JsRuntime`].
//!
//! The system `JavaScriptCore.framework` ships on every Apple platform —
//! iOS, macOS, tvOS, visionOS — so embedding it adds zero binary size. On
//! non-Apple targets this crate compiles to an empty library.
//!
//! [`JsRuntime`]: waterui_ts_engine::JsRuntime

// watchOS has no JavaScriptCore. Rather than silently falling back to a
// bundled engine, the crate states the documented asymmetry at compile time.
#[cfg(target_os = "watchos")]
compile_error!(
    "waterui-ts-engine-jsc is unsupported on watchOS: the platform has no \
     JavaScriptCore. The TypeScript runtime is unavailable on watchOS — use \
     another target."
);

#[cfg(all(target_vendor = "apple", not(target_os = "watchos")))]
mod runtime;

#[cfg(all(target_vendor = "apple", not(target_os = "watchos")))]
pub use runtime::JscRuntime;

#[cfg(all(test, target_vendor = "apple", not(target_os = "watchos")))]
mod tests {
    use crate::JscRuntime;
    use waterui_ts_engine::conformance_tests;

    conformance_tests!(JscRuntime);
}
