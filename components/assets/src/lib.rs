//! Asset management for WaterUI.
//!
//! This crate provides types and utilities for managing assets in WaterUI applications:
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
//! let remote: Data = asset!("https://api.example.com/data.json").await;
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

#[cfg(feature = "std")]
mod bundle;
mod data;
mod error;
mod kind;
mod large_file;
#[cfg(feature = "std")]
mod url;

#[cfg(feature = "std")]
pub use bundle::{
    AudioAsset, Bundle, DataAsset, FontAsset, ImageAsset, LargeFileAsset, VideoAsset,
};
pub use data::Data;
pub use error::AssetError;
pub use kind::AssetKind;
pub use large_file::LargeFile;

/// Prelude for common imports.
pub mod prelude {
    pub use crate::{AssetError, AssetKind, Data, LargeFile};
    #[cfg(feature = "std")]
    pub use crate::{
        AudioAsset, Bundle, DataAsset, FontAsset, ImageAsset, LargeFileAsset, VideoAsset,
    };
}
