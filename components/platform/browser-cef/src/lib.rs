//! Shared Chromium Embedded Framework runtime for `WaterUI`.
//!
//! The standard `waterui-webview` CEF backend and the independent
//! `waterui-chromium` component both use this crate. CEF is dynamically loaded
//! from the packaged application runtime and is never linked by applications
//! that do not select either surface.

mod app;
#[cfg(target_os = "macos")]
mod application_mac;
mod cdp;
mod gpu;
#[cfg(target_os = "macos")]
mod message_pump_macos;
mod page;
mod runtime;
#[cfg(feature = "webview")]
mod webview;

#[cfg(target_os = "macos")]
pub use application_mac::initialize_macos_application;
pub use cdp::CefCdpSession;
pub use gpu::{CefViewport, gpu_view};
#[cfg(target_os = "macos")]
pub use message_pump_macos::CefMacOsMessagePump;
pub use page::{
    AcceleratedFrameSink, CefInputModifiers, CefKeyInput, CefPageHandle, CefPointerButton,
    CefPopupRect, CefTextRange,
};
pub use runtime::{
    CefRuntime, CefRuntimeConfiguration, CefRuntimePaths, PumpDeadline, run_packaged_subprocess,
};
#[cfg(feature = "webview")]
pub use webview::CefWebViewHandle;
