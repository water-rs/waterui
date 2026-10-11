//! Typed image storage formats and the data a commit hands to the render
//! thread for [`Image`](crate::Image) registration.
//!
//! The image-data vocabulary is defined once in `cherenkov-record` — the
//! same types `waterui-graphics` registers — and re-exported here so the
//! engine's `crate::image` paths keep reading the same names.

pub use cherenkov_record::{
    Astc4x4, Bc7, Etc2Rgba, Format, ImageColorSpace, ImageData, ImageFormat, ImageUpload, Rgba8,
    Rgba16F,
};
