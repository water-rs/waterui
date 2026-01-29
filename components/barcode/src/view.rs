use crate::{BarcodeRenderer, BarcodeSource};
use waterui_core::{Environment, Str, View};
use waterui_graphics::GpuSurface;

/// A view that renders a barcode.
///
/// `Barcode` is a high-performance, GPU-accelerated view that renders
/// barcodes and QR codes. It uses compute shaders for pixel-perfect
/// rendering at any scale.
///
/// # Example
///
/// ```rust
/// use waterui_barcode::Barcode;
///
/// Barcode::new("https://waterui.dev");
/// ```
#[derive(Clone, Debug)]
pub struct Barcode {
    content: Str,
}

impl Barcode {
    /// Creates a new barcode view.
    pub fn new(content: impl Into<Str>) -> Self {
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
