//! The QuickJS-NG implementation of [`JsRuntime`].
//!
//! QuickJS-NG is embedded (through `rquickjs`) on every target that is not an
//! Apple platform: Android, Linux, Windows.
//!
//! [`JsRuntime`]: waterui_ts_engine::JsRuntime

mod runtime;

pub use runtime::QuickJsRuntime;

#[cfg(test)]
mod tests {
    use crate::QuickJsRuntime;
    use waterui_ts_engine::conformance_tests;

    conformance_tests!(QuickJsRuntime);
}
