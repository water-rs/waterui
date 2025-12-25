use waterui_core::{Environment, View};
use waterui_graphics::GpuSurface;
use crate::{BarcodeSource, BarcodeRenderer};

/// A view that renders a barcode.
///
/// `Barcode` is a high-performance, GPU-accelerated view that renders
/// barcodes and QR codes. It uses compute shaders for pixel-perfect
/// rendering at any scale.
///
/// # Example
///
/// ```rust
/// Barcode::new("https://waterui.dev")
/// ```
#[derive(Clone, Debug)]
pub struct Barcode {
    content: String,
}

impl Barcode {
    /// Creates a new barcode view.
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
        }
    }
}

impl View for Barcode {
    fn body(self, _env: &Environment) -> impl View {
        // Create source
        let source = BarcodeSource::qr(self.content);
        // Create renderer
        let renderer = BarcodeRenderer::new(source);
        // Create surface
        GpuSurface::new(renderer)
    }
}
