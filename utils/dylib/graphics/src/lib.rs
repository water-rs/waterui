//! Graphics and widget layer of the layered `waterui` dylib chain.
//!
//! Sits on `waterui-dylib-foundation`; carries the GPU stack (`waterui-graphics`,
//! `waterui-shape`, `waterui-svg`, Cherenkov, wgpu) and the widget tier
//! (`layout`, `text`, `form`, `controls`, `navigation`, `icon`), which depends
//! on the graphics crates and therefore cannot sit below them.

pub use waterui_dylib_foundation;

pub use waterui_controls;
pub use waterui_form;
pub use waterui_graphics;
pub use waterui_icon;
pub use waterui_layout;
pub use waterui_navigation;
pub use waterui_shape;
#[cfg(feature = "svg")]
pub use waterui_svg;
pub use waterui_text;
