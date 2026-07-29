//! Bundled WPE `WebKit` engine used by `WaterUI`'s standard Linux `WebView`.
//!
//! The Rust crate never links the host's WPE or `WebKit` libraries. `water`
//! stages an exact runtime next to the application and this crate loads its
//! narrow, versioned `libwaterui_wpe` ABI.

#[cfg(feature = "webview")]
mod abi;
mod frame;
#[cfg(target_os = "linux")]
mod gpu;
#[cfg(feature = "webview")]
mod page;
#[cfg(feature = "webview")]
mod runtime;
#[cfg(feature = "webview")]
mod webview;

#[cfg(feature = "webview")]
pub use frame::WpeFrameLease;
pub use frame::{DmaBufFormat, DmaBufFrame, DmaBufPlane};
#[cfg(all(target_os = "linux", feature = "webview"))]
pub use gpu::WpeGpuView;
#[cfg(target_os = "linux")]
pub use gpu::{DmaBufFrameCopier, DmaBufFrameSource, DmaBufGpuView, WpeViewport};
#[cfg(feature = "webview")]
pub use page::{PointerButton, WpePage};
#[cfg(feature = "webview")]
pub use runtime::{WPE_WEBKIT_VERSION, WpeRuntime, WpeRuntimePaths};
#[cfg(feature = "webview")]
pub use webview::{WpeController, WpeWebViewHandle};
