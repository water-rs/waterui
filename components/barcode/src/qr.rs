//! QR code generation - produces matrix data for GPU rendering.

use waterui_core::Str;

/// A barcode/QR code data source.
///
/// Generates QR matrix data lazily - actual rendering happens on GPU.
pub struct BarcodeSource {
    content: Str,
    /// Cached QR matrix (generated on first access)
    matrix: Option<QrMatrix>,
    /// Output size in pixels
    size: u32,
}

/// QR code matrix data packed for GPU consumption.
#[derive(Debug)]
pub struct QrMatrix {
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

    /// Returns the QR matrix, generating it if needed.
    pub fn matrix(&mut self) -> &QrMatrix {
        if self.matrix.is_none() {
            self.generate_matrix();
        }
        self.matrix.as_ref().expect("Matrix should be generated")
    }

    /// Generates the QR code matrix using fast_qr.
    fn generate_matrix(&mut self) {
        let qr = fast_qr::QRBuilder::new(self.content.as_bytes())
            .build()
            .expect("Failed to generate QR code");

        let dimension = qr.size as u32;
        let total_modules = (dimension * dimension) as usize;

        // Pack modules into u32 words (32 modules per word)
        let num_words = (total_modules + 31) / 32;
        let mut packed_data = vec![0u32; num_words];

        for y in 0..dimension {
            for x in 0..dimension {
                let linear_idx = (y * dimension + x) as usize;
                let is_dark = qr.data.get(linear_idx).map_or(false, |m| m.value());

                if is_dark {
                    let word_idx = linear_idx / 32;
                    let bit_idx = linear_idx % 32;
                    packed_data[word_idx] |= 1u32 << bit_idx;
                }
            }
        }

        self.matrix = Some(QrMatrix {
            dimension,
            packed_data,
        });
    }
}
