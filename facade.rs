#![doc = include_str!("README.md")]

extern crate self as waterui;

// Keep the shared WaterUI runtime in the final link when dynamic linking is enabled.
#[cfg(feature = "dynamic_linking")]
use waterui_dylib as _;

pub use waterui_internal::__export_preview;
pub use waterui_internal::__with_ts_runtime;
#[cfg(any(feature = "ts", feature = "webview"))]
pub use waterui_internal::TsType;
pub use waterui_internal::configure_environment;
pub use waterui_internal::*;
#[cfg(feature = "form")]
pub use waterui_internal::{FormBuilder, form};
#[cfg(feature = "ts")]
pub use waterui_internal::{TsProps, tsx};
#[cfg(feature = "webview")]
pub use waterui_internal::{eval, exec, exec_file, js_api, js_file};
#[cfg(feature = "widgets")]
pub use waterui_internal::{widget, widget_intent};
