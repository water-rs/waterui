//! Shared Chromium Embedded Framework runtime for `WaterUI`.
//!
//! The standard `waterui-webview` CEF backend and the independent
//! `waterui-chromium` component both use this crate. CEF is dynamically loaded
//! from the packaged application runtime and is never linked by applications
//! that do not select either surface.

mod app;
mod cdp;
mod gpu;
mod page;
mod runtime;
#[cfg(feature = "webview")]
mod webview;

pub use cdp::CefCdpSession;
pub use gpu::{CefViewport, gpu_view};
pub use page::{
    AcceleratedFrameSink, CefInputModifiers, CefKeyInput, CefPageHandle, CefPointerButton,
    CefPopupRect,
};
pub use runtime::{
    CefRuntime, CefRuntimeConfiguration, CefRuntimePaths, PumpDeadline, run_packaged_subprocess,
};
#[cfg(feature = "webview")]
pub use webview::CefWebViewHandle;
