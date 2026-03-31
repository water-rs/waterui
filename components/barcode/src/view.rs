use crate::{BarcodeMaskEffect, BarcodeRenderer, BarcodeSource, BarcodeSymbology};
use core::fmt;
use waterui_core::{Environment, Str, View};
use waterui_graphics::{GpuSurface, GpuView, ViewEffect, color::Srgb};

/// Fill style for dark QR modules.
#[derive(Clone, Copy, Debug)]
pub enum BarcodeFill {
    /// Solid color fill.
    Solid(Srgb),
    /// Linear gradient fill in normalized QR-space coordinates.
    LinearGradient {
        /// Gradient start color.
        start_color: Srgb,
        /// Gradient end color.
        end_color: Srgb,
        /// Normalized gradient start point in QR space.
        start_point: [f32; 2],
        /// Normalized gradient end point in QR space.
        end_point: [f32; 2],
    },
}

/// A view that renders a barcode.
///
/// `Barcode` is a high-performance, GPU-accelerated view that renders
/// QR/linear barcodes via fragment shader rasterization from a packed matrix buffer.
///
/// # Example
///
/// ```rust
/// use waterui_barcode::Barcode;
///
/// Barcode::qr("https://waterui.dev");
/// Barcode::code128("HELLO-WATERUI");
/// ```
#[derive(Clone, Debug)]
pub struct Barcode {
    symbology: BarcodeSymbology,
    content: Str,
    fill: BarcodeFill,
    light_color: Srgb,
}

/// Barcode view filled by arbitrary GPU content.
pub struct BarcodeGpuFill<V: GpuView> {
    symbology: BarcodeSymbology,
    content: Str,
    fill: V,
    light_color: Srgb,
}

impl<V: GpuView> fmt::Debug for BarcodeGpuFill<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BarcodeGpuFill").finish_non_exhaustive()
    }
}

impl Barcode {
    /// Creates a new barcode view.
    ///
    /// This is an alias of [`Self::qr`].
    pub fn new(content: impl Into<Str>) -> Self {
        Self::qr(content)
    }

    /// Creates a QR code view.
    pub fn qr(content: impl Into<Str>) -> Self {
        Self {
            symbology: BarcodeSymbology::Qr,
            content: content.into(),
            fill: BarcodeFill::Solid(Srgb::BLACK),
            light_color: Srgb::WHITE,
        }
    }

    /// Creates a Code128 barcode view.
    pub fn code128(content: impl Into<Str>) -> Self {
        Self {
            symbology: BarcodeSymbology::Code128,
            content: content.into(),
            fill: BarcodeFill::Solid(Srgb::BLACK),
            light_color: Srgb::WHITE,
        }
    }

    /// Sets a solid dark module color.
    #[must_use]
    pub fn dark_color(mut self, color: impl Into<Srgb>) -> Self {
        self.fill = BarcodeFill::Solid(color.into());
        self
    }

    /// Sets a linear gradient fill for dark modules.
    ///
    /// Gradient coordinates are normalized to the QR square:
    /// - `[0.0, 0.0]` = top-left
    /// - `[1.0, 1.0]` = bottom-right
    #[must_use]
    pub fn linear_gradient(
        mut self,
        start_color: impl Into<Srgb>,
        end_color: impl Into<Srgb>,
        start_point: [f32; 2],
        end_point: [f32; 2],
    ) -> Self {
        self.fill = BarcodeFill::LinearGradient {
            start_color: start_color.into(),
            end_color: end_color.into(),
            start_point,
            end_point,
        };
        self
    }

    /// Sets the light module/background color.
    #[must_use]
    pub fn light_color(mut self, color: impl Into<Srgb>) -> Self {
        self.light_color = color.into();
        self
    }

    /// Fills dark modules using arbitrary GPU-rendered content.
    ///
    /// Any type implementing `GpuView` can be passed directly because
    /// `GpuView` has a blanket `GpuView` implementation.
    #[must_use]
    pub fn fill_gpu<V: GpuView>(self, fill: V) -> BarcodeGpuFill<V> {
        BarcodeGpuFill {
            symbology: self.symbology,
            content: self.content,
            fill,
            light_color: self.light_color,
        }
    }
}

impl<V: GpuView> BarcodeGpuFill<V> {
    /// Sets light module/background color for the masked QR output.
    #[must_use]
    pub fn light_color(mut self, color: impl Into<Srgb>) -> Self {
        self.light_color = color.into();
        self
    }
}

impl View for Barcode {
    fn body(self, _env: &Environment) -> impl View {
        let source = match self.symbology {
            BarcodeSymbology::Qr => BarcodeSource::qr(self.content),
            BarcodeSymbology::Code128 => BarcodeSource::code128(self.content),
        };
        let renderer = BarcodeRenderer::new_with_fill(source, self.fill, self.light_color);
        GpuSurface::new(renderer)
    }
}

impl<V: GpuView> View for BarcodeGpuFill<V> {
    fn body(self, _env: &Environment) -> impl View {
        let source = match self.symbology {
            BarcodeSymbology::Qr => BarcodeSource::qr(self.content),
            BarcodeSymbology::Code128 => BarcodeSource::code128(self.content),
        };
        let effect = BarcodeMaskEffect::new(source, self.light_color);
        let fill_surface = GpuSurface::new(self.fill);
        ViewEffect::new(fill_surface, effect)
    }
}
