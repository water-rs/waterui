//! Barcode matrix generation for GPU rendering.

use barcoders::sym::code128::Code128;
use core::fmt;
use waterui_core::Str;
use waterui_graphics::{GpuRuntime, GpuSurface, OffscreenRenderConfig, OffscreenSize};

use crate::BarcodeRenderer;

/// Supported barcode symbologies.
///
/// This enum is `#[non_exhaustive]` so that future symbologies (EAN-13,
/// `DataMatrix`, PDF417, …) can be added without breaking downstream `match`
/// statements. Pattern matches against this type must include a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BarcodeSymbology {
    /// 2D QR code matrix.
    Qr,
    /// 1D Code128 barcode.
    Code128,
}

/// A barcode data source.
///
/// Encodes module data once during construction; rasterization happens on GPU.
#[derive(Clone)]
pub struct BarcodeSource {
    symbology: BarcodeSymbology,
    matrix: BarcodeMatrix,
    /// Output size in pixels
    size: u32,
}

/// Barcode matrix data packed for GPU consumption.
#[derive(Debug, Clone)]
pub struct BarcodeMatrix {
    /// Matrix width in modules.
    pub width: u32,
    /// Matrix height in modules.
    ///
    /// Two-dimensional symbologies use their encoded row count. Linear
    /// symbologies use one row because bar height is a rendering concern.
    pub height: u32,
    /// Packed matrix data - each u32 contains 32 modules as bits
    /// Bit 0 of word 0 = module (0,0), bit 1 = module (1,0), etc.
    /// 1 = dark module, 0 = light module
    pub packed_data: Vec<u32>,
}

impl fmt::Debug for BarcodeSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BarcodeSource")
            .field("symbology", &self.symbology)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl BarcodeSource {
    /// Creates a new QR code from content.
    #[must_use]
    pub fn qr(content: impl Into<Str>) -> Self {
        let content = content.into();
        Self::new(BarcodeSymbology::Qr, content.as_ref())
    }

    /// Creates a new Code128 barcode from content.
    #[must_use]
    pub fn code128(content: impl Into<Str>) -> Self {
        let content = content.into();
        Self::new(BarcodeSymbology::Code128, content.as_ref())
    }

    fn new(symbology: BarcodeSymbology, content: &str) -> Self {
        let matrix = match symbology {
            BarcodeSymbology::Qr => Self::generate_qr_matrix(content),
            BarcodeSymbology::Code128 => Self::generate_code128_matrix(content),
        };
        Self {
            symbology,
            matrix,
            size: 256,
        }
    }

    /// Sets the output size in pixels.
    pub const fn set_size(&mut self, size: u32) {
        self.size = size;
    }

    /// Returns the output size.
    #[must_use]
    pub const fn size(&self) -> u32 {
        self.size
    }

    /// Returns source symbology.
    #[must_use]
    pub const fn symbology(&self) -> BarcodeSymbology {
        self.symbology
    }

    /// Returns quiet-zone width in modules.
    #[must_use]
    pub const fn quiet_zone(&self) -> u32 {
        match self.symbology {
            BarcodeSymbology::Qr => 4,
            BarcodeSymbology::Code128 => 10,
        }
    }

    pub(crate) const fn vertical_quiet_zone(&self) -> u32 {
        match self.symbology {
            BarcodeSymbology::Qr => self.quiet_zone(),
            BarcodeSymbology::Code128 => 0,
        }
    }

    pub(crate) const fn preserves_square_modules(&self) -> bool {
        match self.symbology {
            BarcodeSymbology::Qr => true,
            BarcodeSymbology::Code128 => false,
        }
    }

    /// Returns the encoded matrix.
    ///
    /// Visible only inside the barcode crate; external code should obtain a
    /// rendered barcode through [`crate::Barcode`] or [`crate::BarcodeRenderer`]
    /// rather than poking at the cached matrix.
    ///
    pub(crate) const fn matrix(&self) -> &BarcodeMatrix {
        &self.matrix
    }

    fn generate_qr_matrix(content: &str) -> BarcodeMatrix {
        let qr = fast_qr::QRBuilder::new(content.as_bytes())
            .build()
            .unwrap_or_else(|error| panic!("failed to encode QR content: {error:?}"));

        let dimension = u32::try_from(qr.size)
            .expect("BarcodeSource::generate_qr_matrix: QR size must fit into u32");
        let total_modules = (dimension * dimension) as usize;
        let num_words = total_modules.div_ceil(32);
        let mut packed_data = vec![0u32; num_words];

        for y in 0..dimension {
            for x in 0..dimension {
                let linear_idx = (y * dimension + x) as usize;
                if qr.data.get(linear_idx).is_some_and(|m| m.value()) {
                    let word_idx = linear_idx / 32;
                    let bit_idx = linear_idx % 32;
                    packed_data[word_idx] |= 1u32 << bit_idx;
                }
            }
        }

        BarcodeMatrix {
            width: dimension,
            height: dimension,
            packed_data,
        }
    }

    fn render_dimensions(&self) -> (u32, u32) {
        let quiet_zone = self.quiet_zone();
        let configured_size = self.size;
        let matrix = self.matrix();
        match self.symbology {
            BarcodeSymbology::Qr => {
                let extent = matrix.width + quiet_zone * 2;
                let target_size = configured_size.max(extent);
                (target_size, target_size)
            }
            BarcodeSymbology::Code128 => {
                let width = configured_size.max(matrix.width + quiet_zone * 2);
                (width, configured_size)
            }
        }
    }

    fn generate_code128_matrix(content: &str) -> BarcodeMatrix {
        // Barcoders requires an explicit start charset marker; default to charset B.
        let payload = match content.chars().next() {
            Some('À' | 'Ɓ' | 'Ć') => content.to_string(),
            _ => format!("Ɓ{content}"),
        };
        let encoded = Code128::new(payload)
            .unwrap_or_else(|error| panic!("failed to encode Code128 content: {error:?}"))
            .encode();
        assert!(!encoded.is_empty(), "Code128 encoder returned no modules");

        let width = u32::try_from(encoded.len())
            .expect("BarcodeSource::generate_code128_matrix: encoded length must fit into u32");
        let mut packed_data = vec![0u32; encoded.len().div_ceil(32)];

        for (linear_idx, module) in encoded.into_iter().enumerate() {
            if module == 1 {
                let word_idx = linear_idx / 32;
                let bit_idx = linear_idx % 32;
                packed_data[word_idx] |= 1u32 << bit_idx;
            }
        }

        BarcodeMatrix {
            width,
            height: 1,
            packed_data,
        }
    }
}

impl waterui_graphics::image_generator::ImageGenerator for BarcodeSource {
    #[expect(
        clippy::future_not_send,
        reason = "barcode generation awaits the UI-local offscreen GpuView environment"
    )]
    async fn generate(
        &self,
        runtime: &GpuRuntime,
    ) -> waterui_graphics::image_generator::GeneratedImage {
        let (width, height) = self.render_dimensions();
        let size = OffscreenSize::try_from_pixels(width, height)
            .expect("BarcodeSource::generate: dimensions must be non-zero");
        let config = OffscreenRenderConfig::new(size).format(wgpu::TextureFormat::Rgba8Unorm);
        let mut env = waterui_core::Environment::new();
        let output = GpuSurface::new(BarcodeRenderer::new(self.clone()))
            .render_offscreen(runtime, config, &mut env)
            .await
            .expect("BarcodeSource::generate: GPU offscreen render should succeed");
        waterui_graphics::image_generator::GeneratedImage::from_rgba8(
            output.width,
            output.height,
            output.rgba8,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui_graphics::image_generator::ImageGenerator;

    #[test]
    fn code128_matrix_stores_one_row_of_modules() {
        let source = BarcodeSource::code128("HELLO-WATERUI-128");
        let matrix = source.matrix();

        assert_eq!(matrix.height, 1);
        assert_eq!(matrix.packed_data.len(), matrix.width.div_ceil(32) as usize);
    }

    #[test]
    fn qr_matrix_remains_square() {
        let source = BarcodeSource::qr("https://waterui.dev");
        let matrix = source.matrix();

        assert_eq!(matrix.width, matrix.height);
        assert_eq!(
            matrix.packed_data.len(),
            (matrix.width * matrix.height).div_ceil(32) as usize
        );
    }

    #[test]
    fn qr_generator_produces_expected_size_and_pixels() {
        let runtime = pollster::block_on(GpuRuntime::new())
            .expect("barcode tests require a working GPU runtime");
        let mut source = BarcodeSource::qr("https://waterui.dev");
        source.set_size(192);

        let image = pollster::block_on(source.generate(&runtime));

        assert_eq!(image.width(), 192);
        assert_eq!(image.height(), 192);
        assert_eq!(image.rgba8().len(), 192 * 192 * 4);
    }

    #[test]
    fn code128_generator_produces_expected_size_and_pixels() {
        let runtime = pollster::block_on(GpuRuntime::new())
            .expect("barcode tests require a working GPU runtime");
        let mut source = BarcodeSource::code128("HELLO-WATERUI");
        source.set_size(256);

        let image = pollster::block_on(source.generate(&runtime));

        assert!(image.width() >= 256);
        assert_eq!(image.width(), image.height());
        assert_eq!(
            image.rgba8().len(),
            image.width() as usize * image.height() as usize * 4
        );
    }
}
