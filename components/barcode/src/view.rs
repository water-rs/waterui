use crate::{BarcodeMaskEffect, BarcodeRenderer, BarcodeSource, BarcodeSymbology};
use core::fmt;
use waterui_core::{Environment, Str, View, layout::UnitPoint};
use waterui_graphics::{GpuSurface, GpuView, ViewEffect, color::Color};

/// Fill style for dark barcode modules.
///
/// Colors are kept as the unresolved [`Color`] type so that barcode views
/// participate in the same theme/HDR resolution flow as the rest of the
/// component library. Resolution to GPU-ready linear RGB happens inside the
/// renderer's setup pass when an [`Environment`] is available.
#[derive(Clone, Debug)]
pub enum BarcodeFill {
    /// Solid color fill.
    Solid(Color),
    /// Linear gradient fill in normalized barcode-space coordinates.
    LinearGradient {
        /// Gradient start color.
        start_color: Color,
        /// Gradient end color.
        end_color: Color,
        /// Normalized gradient start point in barcode space.
        start_point: UnitPoint,
        /// Normalized gradient end point in barcode space.
        end_point: UnitPoint,
    },
}

impl BarcodeFill {
    fn solid_default() -> Self {
        Self::Solid(Color::from(waterui_graphics::color::Srgb::BLACK))
    }
}

fn default_light_color() -> Color {
    Color::from(waterui_graphics::color::Srgb::WHITE)
}

/// A view that renders a barcode.
///
/// `Barcode` is a high-performance, GPU-accelerated view that renders
/// QR/linear barcodes via fragment shader rasterization from a packed matrix
/// buffer.
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
    light_color: Color,
}

/// Barcode view filled by arbitrary GPU content.
pub struct BarcodeGpuFill<V: GpuView> {
    symbology: BarcodeSymbology,
    content: Str,
    fill: V,
    light_color: Color,
}

impl<V: GpuView> fmt::Debug for BarcodeGpuFill<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BarcodeGpuFill").finish_non_exhaustive()
    }
}

impl Barcode {
    /// Creates a QR code view.
    pub fn qr(content: impl Into<Str>) -> Self {
        Self {
            symbology: BarcodeSymbology::Qr,
            content: content.into(),
            fill: BarcodeFill::solid_default(),
            light_color: default_light_color(),
        }
    }

    /// Creates a Code128 barcode view.
    pub fn code128(content: impl Into<Str>) -> Self {
        Self {
            symbology: BarcodeSymbology::Code128,
            content: content.into(),
            fill: BarcodeFill::solid_default(),
            light_color: default_light_color(),
        }
    }

    /// Sets a solid dark module color.
    #[must_use]
    pub fn dark_color(mut self, color: impl Into<Color>) -> Self {
        self.fill = BarcodeFill::Solid(color.into());
        self
    }

    /// Sets a linear gradient fill for dark modules.
    ///
    /// Gradient coordinates are normalized to the barcode square via
    /// [`UnitPoint`]: `UnitPoint::TOP_LEADING` = top-left,
    /// `UnitPoint::BOTTOM_TRAILING` = bottom-right.
    #[must_use]
    pub fn linear_gradient(
        mut self,
        start_color: impl Into<Color>,
        end_color: impl Into<Color>,
        start_point: impl Into<UnitPoint>,
        end_point: impl Into<UnitPoint>,
    ) -> Self {
        self.fill = BarcodeFill::LinearGradient {
            start_color: start_color.into(),
            end_color: end_color.into(),
            start_point: start_point.into(),
            end_point: end_point.into(),
        };
        self
    }

    /// Sets the light module/background color.
    #[must_use]
    pub fn light_color(mut self, color: impl Into<Color>) -> Self {
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
    /// Sets light module/background color for the masked barcode output.
    #[must_use]
    pub fn light_color(mut self, color: impl Into<Color>) -> Self {
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
        let renderer = BarcodeRenderer::new(source)
            .with_fill(self.fill)
            .with_light_color(self.light_color);
        GpuSurface::new(renderer)
    }
}

impl<V: GpuView> View for BarcodeGpuFill<V> {
    fn body(self, env: &Environment) -> impl View {
        use waterui_core::Signal;

        let source = match self.symbology {
            BarcodeSymbology::Qr => BarcodeSource::qr(self.content),
            BarcodeSymbology::Code128 => BarcodeSource::code128(self.content),
        };
        let resolved_light = self.light_color.resolve(env).get();
        let effect = BarcodeMaskEffect::new(source, resolved_light);
        let fill_surface = GpuSurface::new(self.fill);
        ViewEffect::new(fill_surface, effect)
    }
}

/// Free-function entry point for a QR code view.
#[must_use]
pub fn qr_code(content: impl Into<Str>) -> Barcode {
    Barcode::qr(content)
}

/// Free-function entry point for a Code128 barcode view.
#[must_use]
pub fn code128(content: impl Into<Str>) -> Barcode {
    Barcode::code128(content)
}
