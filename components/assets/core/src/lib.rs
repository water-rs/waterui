//! Media-free asset primitives for `WaterUI`.
//!
//! This crate holds the pieces of asset management that carry no `View`,
//! media, or rendering dependency, so tools like the `WaterUI` CLI and the
//! asset planner can classify assets and perform asset I/O without compiling
//! the runtime media stack:
//!
//! - [`AssetKind`] - Asset type classification based on file extension
//! - [`AssetError`] - Error types for asset operations
//! - [`download_remote_bytes`] / [`write_bytes_atomically`] - Remote fetch and
//!   atomic file writes (std, non-wasm)
//! - [`WINDOW_ICON_FILE`] - File name of the staged runtime window icon
//!
//! The application-facing types (`Data`, `LargeFile`, `Bundle`) live in
//! `waterui-assets`, which depends on this crate and re-exports everything
//! here.

#![no_std]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

mod error;
mod kind;
#[cfg(all(feature = "std", not(target_arch = "wasm32")))]
mod remote;
#[cfg(all(feature = "std", target_arch = "wasm32"))]
mod remote_web;
#[cfg(feature = "std")]
mod url;

/// File name of the staged runtime window icon inside the asset bundle.
pub const WINDOW_ICON_FILE: &str = ".window-icon.png";

pub use error::AssetError;
pub use kind::AssetKind;
#[cfg(all(feature = "std", not(target_arch = "wasm32")))]
pub use remote::{AtomicWriteOutcome, download_remote_bytes, write_bytes_atomically};
#[cfg(all(feature = "std", target_arch = "wasm32"))]
pub use remote_web::download_remote_bytes;
#[cfg(feature = "std")]
pub use url::{ensure_http_allowed, is_loopback_http_url, is_remote_url};
