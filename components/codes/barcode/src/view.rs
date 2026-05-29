use crate::{BarcodeMaskEffect, BarcodeRenderer, BarcodeSource, BarcodeSymbology};
use core::fmt;
use waterui_core::{AnyView, Environment, Signal, Str, View, layout::UnitPoint};
use waterui_graphics::{
    GpuSurface, GpuView, ViewEffect,
    color::{Color, ResolvedColor},
};
use waterui_image::{Image, Interpolation};

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
    fn body(self, env: &Environment) -> impl View {
        let Self {
            symbology,
            content,
            fill,
            light_color,
        } = self;
        let mut source = match symbology {
            BarcodeSymbology::Qr => BarcodeSource::qr(content),
            BarcodeSymbology::Code128 => BarcodeSource::code128(content),
        };
        match fill {
            // Solid fills are tiny per-module bitmaps that the GPU sampler
            // can scale via nearest-neighbor — no need to spin the full
            // packed-matrix fragment shader for every frame.
            BarcodeFill::Solid(dark) => {
                let resolved_dark = dark.resolve(env).get();
                let resolved_light = light_color.resolve(env).get();
                AnyView::new(render_solid_bitmap(
                    &mut source,
                    resolved_dark,
                    resolved_light,
                ))
            }
            // Gradients still need shader interpolation across modules, so
            // they keep the original GPU rasterizer path.
            gradient @ BarcodeFill::LinearGradient { .. } => {
                let renderer = BarcodeRenderer::new(source)
                    .with_fill(gradient)
                    .with_light_color(light_color);
                AnyView::new(GpuSurface::new(renderer))
            }
        }
    }
}

fn render_solid_bitmap(
    source: &mut BarcodeSource,
    dark: ResolvedColor,
    light: ResolvedColor,
) -> Image {
    let quiet_zone = source.quiet_zone();
    let matrix = source.matrix();
    let dim = matrix.dimension;
    let total = dim + 2 * quiet_zone;
    let total_usize = total as usize;
    let dark_rgba = resolved_to_rgba8(dark);
    let light_rgba = resolved_to_rgba8(light);
    let mut pixels = vec![0u8; total_usize * total_usize * 4];
    for y in 0..total {
        for x in 0..total {
            let module = match (x.checked_sub(quiet_zone), y.checked_sub(quiet_zone)) {
                (Some(mx), Some(my)) if mx < dim && my < dim => {
                    let linear_idx = (my * dim + mx) as usize;
                    let word = matrix.packed_data[linear_idx / 32];
                    (word >> (linear_idx % 32)) & 1 == 1
                }
                _ => false,
            };
            let rgba = if module { dark_rgba } else { light_rgba };
            let offset = (y as usize * total_usize + x as usize) * 4;
            pixels[offset..offset + 4].copy_from_slice(&rgba);
        }
    }
    Image::new(pixels, total, total)
        .interpolation(Interpolation::Nearest)
        .resizable()
}

fn resolved_to_rgba8(color: ResolvedColor) -> [u8; 4] {
    let srgb = color.to_srgb();
    let to_byte = |v: f32| -> u8 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let scaled = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        scaled
    };
    [
        to_byte(srgb.red),
        to_byte(srgb.green),
        to_byte(srgb.blue),
        to_byte(color.opacity),
    ]
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
