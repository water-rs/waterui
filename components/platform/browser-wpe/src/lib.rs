//! Bundled WPE `WebKit` engine used by `WaterUI`'s standard Linux `WebView`.
//!
//! The Rust crate never links the host's WPE or `WebKit` libraries. `water`
//! stages an exact runtime next to the application and this crate loads its
//! narrow, versioned `libwaterui_wpe` ABI.
//!
//! WPE is a Linux engine — the runtime polls dma-buf fences through `poll(2)`
//! and passes dma-buf file descriptors — so this crate is empty elsewhere,
//! the same way `waterui-gtk` is.
//!
//! The frames themselves are [`wgpu_external_frame::dma_buf::DmaBufFrame`]s:
//! importing a dma-buf into a `wgpu` texture is a general problem, so it lives
//! in a crate that knows nothing about WPE or `WaterUI`. What is WPE's is the
//! buffer lease ([`WpeFrameLease`]) and the compositing view built on top
//! ([`DmaBufGpuView`]).
//!
//! # Testing against the real engine
//!
//! `tests/real_engine.rs` drives an actual WPE `WebKit` runtime — navigation,
//! history, and the `waterui` bridge in both directions. Running it needs a
//! staged runtime, so it sits behind the `real-engine` feature and its module
//! documentation carries the commands. `.github/workflows/browser-wpe.yml` runs
//! it on the paths it guards; nothing else in CI does.

#[cfg(all(feature = "webview", target_os = "linux"))]
mod abi;
#[cfg(all(feature = "webview", target_os = "linux"))]
mod frame;
#[cfg(target_os = "linux")]
mod gpu;
#[cfg(all(feature = "webview", target_os = "linux"))]
mod page;
#[cfg(all(feature = "webview", target_os = "linux"))]
mod runtime;
#[cfg(all(feature = "webview", target_os = "linux"))]
mod webview;

#[cfg(all(feature = "webview", target_os = "linux"))]
pub use frame::WpeFrameLease;
#[cfg(all(target_os = "linux", feature = "webview"))]
pub use gpu::WpeGpuView;
#[cfg(target_os = "linux")]
pub use gpu::{DmaBufFrameSource, DmaBufGpuView, WpeViewport};
#[cfg(all(feature = "webview", target_os = "linux"))]
pub use page::{PointerButton, WpePage};
#[cfg(all(feature = "webview", target_os = "linux"))]
pub use runtime::{WPE_WEBKIT_VERSION, WpeRuntime, WpeRuntimePaths};
#[cfg(all(feature = "webview", target_os = "linux"))]
pub use webview::{WpeController, WpeWebViewHandle};
