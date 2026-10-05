//! Dynamic-link anchor crate for `waterui`.
//!
//! The layers below are `pub use`d so their DLLs land in this image's
//! dependency set: the crates they embed are then satisfied through those DLLs
//! instead of being embedded a second time here (#1615).

extern crate waterui_internal;

#[cfg(any(
    feature = "canvas",
    feature = "chart",
    feature = "barcode",
    feature = "particle"
))]
pub use waterui_dylib_components;
pub use waterui_dylib_foundation;
pub use waterui_dylib_graphics;
#[cfg(any(feature = "flow-markdown", feature = "markdown-math"))]
pub use waterui_dylib_markdown;
#[cfg(any(feature = "media", feature = "video", feature = "video-gpu"))]
pub use waterui_dylib_media;
#[cfg(feature = "webview")]
pub use waterui_dylib_webview;
pub use waterui_dylib_widgets;
