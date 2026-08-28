//! Full Chromium integration for `WaterUI`.
//!
//! This crate is deliberately separate from `waterui-webview`. It exposes visible
//! Chromium views, headless pages, raw CDP, and generated strongly typed CDP without
//! making Chromium part of the standard `WebView` dependency graph.

mod cdp_session;
mod controller;
mod page;

pub use cdp_session::{CdpError, CdpEvent, CdpSession, CustomCdpSession};
pub use chromiumoxide_cdp::cdp;
pub use chromiumoxide_types as cdp_types;
pub use controller::{ChromiumController, CustomChromiumController};
pub use page::{
    AnyChromiumPageHandle, Chromium, ChromiumConfiguration, ChromiumEvent, ChromiumPage,
    ChromiumPageHandle, ChromiumProfile, ChromiumProxy, ChromiumView, PageMode, ScreenshotFormat,
    chromium,
};
