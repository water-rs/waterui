//! Asset management for `WaterUI`.
//!
//! This crate provides types and utilities for managing assets in `WaterUI` applications:
//!
//! - [`Data`] - Small binary files loaded into memory (configs, shaders, JSON)
//! - [`LargeFile`] - Large files using memory-mapping (ML models, large binaries)
//! - [`AssetKind`] - Asset type classification based on file extension
//! - [`AssetError`] - Error types for asset operations
//!
//! # Usage with `asset!` macro
//!
//! The `asset!` macro (from `waterui-assets-macros`) provides compile-time type inference:
//!
//! ```ignore
//! // Media types (Photo, Video, Audio) - sync, URL-based
//! let photo: Photo = asset!("logo.png");
//! let video: Video = asset!("intro.mp4");
//!
//! // Data type - sync for local, async for remote
//! let config: Data = asset!("config.json");
//! let remote: Data =
//!     asset!("https://raw.githubusercontent.com/water-rs/waterui/dev/Cargo.toml").await;
//!
//! // LargeFile - always async (mmap setup required)
//! let model: LargeFile = asset!("model.onnx").await;
//! model.warm().await;  // Pre-warm pages before access
//! ```
//!
//! # Memory-Mapped Files Warning
//!
//! [`LargeFile`] uses memory-mapping for efficient access to large files.
//! However, accessing data without warming may cause blocking due to page faults.
//! Under memory pressure, even warmed pages may be evicted by the OS.
//!
//! **Recommendation**: Use [`LargeFile`] on background threads for best performance.

#![no_std]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

#[cfg(all(feature = "std", not(target_arch = "wasm32")))]
mod bundle;
mod data;
mod error;
mod kind;
#[cfg(all(feature = "std", not(target_arch = "wasm32")))]
mod large_file;
#[cfg(all(feature = "std", not(target_arch = "wasm32")))]
mod remote;
#[cfg(all(feature = "std", target_arch = "wasm32"))]
mod remote_web;
#[cfg(feature = "std")]
mod url;

#[cfg(all(feature = "std", not(target_arch = "wasm32")))]
pub use bundle::{
    AudioAsset, Bundle, DataAsset, FontAsset, ImageAsset, LargeFileAsset, VideoAsset,
};
pub use data::Data;
pub use error::AssetError;
pub use kind::AssetKind;
#[cfg(all(feature = "std", not(target_arch = "wasm32")))]
pub use large_file::LargeFile;
#[cfg(all(feature = "std", not(target_arch = "wasm32")))]
pub use remote::{AtomicWriteOutcome, download_remote_bytes, write_bytes_atomically};
#[cfg(all(feature = "std", target_arch = "wasm32"))]
pub use remote_web::download_remote_bytes;
#[cfg(feature = "std")]
pub use url::{ensure_http_allowed, is_loopback_http_url, is_remote_url};

/// Prelude for common imports.
pub mod prelude {
    pub use crate::{AssetError, AssetKind, Data};
    #[cfg(all(feature = "std", not(target_arch = "wasm32")))]
    pub use crate::{
        AudioAsset, Bundle, DataAsset, FontAsset, ImageAsset, LargeFile, LargeFileAsset, VideoAsset,
    };
}
