//! View renderer utilities for preview.
//!
//! This module re-exports the `ViewRenderer` from `waterui-core` and adds
//! PNG encoding functionality for the preview system.

pub use waterui_core::view_renderer::{
    CustomViewRenderer, RenderError, RenderResult, RenderSize, ViewRenderer,
};

/// Extension trait for `RenderResult` to add PNG encoding.
pub trait RenderResultExt {
    /// Encode the RGBA data as PNG, consuming the render result.
    ///
    /// # Errors
    ///
    /// Returns an error if the capture is empty or PNG encoding fails.
    fn into_png(self) -> Result<Vec<u8>, waterui_preview_protocol::run::PngError>;

    /// Encode the RGBA data as PNG.
    ///
    /// # Errors
    ///
    /// Returns an error if the capture is empty or PNG encoding fails.
    fn to_png(&self) -> Result<Vec<u8>, waterui_preview_protocol::run::PngError>;
}

impl RenderResultExt for RenderResult {
    fn into_png(self) -> Result<Vec<u8>, waterui_preview_protocol::run::PngError> {
        waterui_preview_protocol::run::encode_png(self.width, self.height, self.rgba_data)
    }

    fn to_png(&self) -> Result<Vec<u8>, waterui_preview_protocol::run::PngError> {
        waterui_preview_protocol::run::encode_png(self.width, self.height, self.rgba_data.clone())
    }
}
