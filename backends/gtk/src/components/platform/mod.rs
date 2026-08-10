//! GTK widget implementations for platform-backed `WaterUI` components.

#[cfg(feature = "chromium")]
pub mod chromium;
pub mod dynamic;
pub mod system_icon;
#[cfg(any(
    feature = "webview-default",
    feature = "webview-system",
    feature = "webview-wpe",
    feature = "webview-cef"
))]
pub mod webview;
