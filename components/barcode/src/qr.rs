//! Barcode matrix generation for GPU rendering.

use barcoders::sym::code128::Code128;
use waterui_core::Str;
use waterui_graphics::{GpuSurface, OffscreenRenderConfig, OffscreenSize};

use crate::BarcodeRenderer;

/// Supported barcode symbologies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarcodeSymbology {
    /// 2D QR code matrix.
    Qr,
    /// 1D Code128 barcode.
    Code128,
}

/// A barcode data source.
///
/// Generates QR matrix data lazily - actual rendering happens on GPU.
#[derive(Clone)]
pub struct BarcodeSource {
    symbology: BarcodeSymbology,
    content: Str,
    /// Cached barcode matrix (generated on first access)
    matrix: Option<BarcodeMatrix>,
    /// Output size in pixels
    size: u32,
}

/// Barcode matrix data packed for GPU consumption.
#[derive(Debug, Clone)]
pub struct BarcodeMatrix {
    /// Matrix dimension (number of modules per side)
    pub dimension: u32,
    /// Packed matrix data - each u32 contains 32 modules as bits
    /// Bit 0 of word 0 = module (0,0), bit 1 = module (1,0), etc.
    /// 1 = dark module, 0 = light module
    pub packed_data: Vec<u32>,
}

impl core::fmt::Debug for BarcodeSource {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BarcodeSource")
            .field("symbology", &self.symbology)
            .field("content", &self.content)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl BarcodeSource {
    /// Creates a new QR code from content.
    #[must_use]
    pub fn qr(content: impl Into<Str>) -> Self {
        Self {
            symbology: BarcodeSymbology::Qr,
            content: content.into(),
            matrix: None,
            size: 256, // Default size
        }
    }

    /// Creates a new Code128 barcode from content.
    #[must_use]
    pub fn code128(content: impl Into<Str>) -> Self {
        Self {
            symbology: BarcodeSymbology::Code128,
            content: content.into(),
            matrix: None,
            size: 256, // Default size
        }
    }

    /// Sets the output size in pixels.
    pub fn set_size(&mut self, size: u32) {
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

    /// Returns the encoded matrix, generating it if needed.
    pub fn matrix(&mut self) -> &BarcodeMatrix {
        if self.matrix.is_none() {
            self.generate_matrix();
        }
        self.matrix.as_ref().expect("Matrix should be generated")
    }

    /// Generates a packed matrix based on configured symbology.
    fn generate_matrix(&mut self) {
        self.matrix = Some(match self.symbology {
            BarcodeSymbology::Qr => Self::generate_qr_matrix(self.content.as_ref()),
            BarcodeSymbology::Code128 => Self::generate_code128_matrix(self.content.as_ref()),
        });
    }

    fn generate_qr_matrix(content: &str) -> BarcodeMatrix {
        let Ok(qr) = fast_qr::QRBuilder::new(content.as_bytes()).build() else {
            return BarcodeMatrix::empty();
        };

        let dimension = qr.size as u32;
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
            dimension,
            packed_data,
        }
    }

    fn render_dimensions(&mut self) -> (u32, u32) {
        let quiet_zone = self.quiet_zone();
        let configured_size = self.size;
        let matrix = self.matrix();
        let extent = matrix.dimension + quiet_zone * 2;
        let target_size = configured_size.max(extent);
        (target_size, target_size)
    }

    fn generate_code128_matrix(content: &str) -> BarcodeMatrix {
        // Barcoders requires an explicit start charset marker; default to charset B.
        let payload = match content.chars().next() {
            Some('À' | 'Ɓ' | 'Ć') => content.to_string(),
            _ => format!("Ɓ{content}"),
        };
        let Ok(encoded) = Code128::new(payload).map(|code| code.encode()) else {
            return BarcodeMatrix::empty();
        };
        if encoded.is_empty() {
            return BarcodeMatrix::empty();
        }

        // Keep current square-matrix shader path: repeat 1D bars on every row.
        let dimension = encoded.len() as u32;
        let total_modules = (dimension * dimension) as usize;
        let num_words = total_modules.div_ceil(32);
        let mut packed_data = vec![0u32; num_words];

        for y in 0..dimension {
            for x in 0..dimension {
                let linear_idx = (y * dimension + x) as usize;
                if encoded[x as usize] == 1 {
                    let word_idx = linear_idx / 32;
                    let bit_idx = linear_idx % 32;
                    packed_data[word_idx] |= 1u32 << bit_idx;
                }
            }
        }

        BarcodeMatrix {
            dimension,
            packed_data,
        }
    }
}

impl BarcodeMatrix {
    /// Creates an all-light fallback matrix used when encoding fails.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            dimension: 1,
            packed_data: vec![0],
        }
    }
}

impl waterui_graphics::image_generator::ImageGenerator for BarcodeSource {
    fn generate(&mut self) -> waterui_graphics::image_generator::GeneratedImage {
        let (width, height) = self.render_dimensions();
        let size = OffscreenSize::try_from_pixels(width, height)
            .expect("BarcodeSource::generate: dimensions must be non-zero");
        let config = OffscreenRenderConfig::new(size).format(wgpu::TextureFormat::Rgba8Unorm);
        let mut env = waterui_core::Environment::new();
        let output = GpuSurface::new(BarcodeRenderer::new(self.clone()))
            .render_offscreen(config, &mut env)
            .expect("BarcodeSource::generate: GPU offscreen render should succeed");
        waterui_graphics::image_generator::GeneratedImage::from_rgba8(
            output.width,
            output.height,
            output.rgba8,
        )
    }
}

/// Backward-compatible alias for previous QR-focused matrix name.
pub type QrMatrix = BarcodeMatrix;

#[cfg(test)]
mod tests {
    use super::*;
    use waterui_graphics::image_generator::ImageGenerator;

    #[test]
    fn qr_generator_produces_expected_size_and_pixels() {
        let mut source = BarcodeSource::qr("https://waterui.dev");
        source.set_size(192);

        let image = source.generate();

        assert_eq!(image.width(), 192);
        assert_eq!(image.height(), 192);
        assert_eq!(image.rgba8().len(), 192 * 192 * 4);
        assert!(image.rgba8().chunks_exact(4).any(|px| px[0] == 0));
        assert!(image.rgba8().chunks_exact(4).any(|px| px[0] == 255));
    }

    #[test]
    fn code128_generator_produces_expected_size_and_pixels() {
        let mut source = BarcodeSource::code128("HELLO-WATERUI");
        source.set_size(256);

        let image = source.generate();

        assert!(image.width() >= 256);
        assert_eq!(image.width(), image.height());
        assert_eq!(
            image.rgba8().len(),
            image.width() as usize * image.height() as usize * 4
        );
        assert!(image.rgba8().chunks_exact(4).any(|px| px[0] == 0));
        assert!(image.rgba8().chunks_exact(4).any(|px| px[0] == 255));
    }
}
