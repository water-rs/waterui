//! Graphics layer of the layered `waterui` dylib chain.
//!
//! Sits on `waterui-dylib-foundation`; carries the GPU stack (`waterui-graphics`,
//! `waterui-shape`, `waterui-svg`, Cherenkov, wgpu). The widget tier lives one
//! layer up in `waterui-dylib-widgets` — it depends on this image.

pub use waterui_dylib_foundation;

pub use waterui_graphics;
pub use waterui_shape;
#[cfg(feature = "svg")]
pub use waterui_svg;
