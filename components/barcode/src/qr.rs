//! QR code generation and Source implementation.

use filtrate_core::{RenderContext, Source};

/// A barcode or QR code source for GPU rendering.
///
/// Generates barcode data lazily and renders via compute shader.
pub struct BarcodeSource {
    content: String,
    // Cached QR matrix (generated on first resolve)
    matrix: Option<QrMatrix>,
    // GPU resources
    texture: Option<wgpu::Texture>,
    dirty: bool,
    // Configuration
    size: u32,
}

/// Internal representation of the QR code matrix.
struct QrMatrix {
    data: Vec<bool>,
    dimension: usize,
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
    /// Creates a new QR code barcode from content.
    #[must_use]
    pub fn qr(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            matrix: None,
            texture: None,
            dirty: true,
            size: 256, // Default size
        }
    }
    
    /// Sets the output size in pixels.
    pub fn set_size(&mut self, size: u32) {
        if self.size != size {
            self.size = size;
            self.texture = None; // Invalidate texture
            self.dirty = true;
        }
    }

    /// Generates the QR code matrix if not cached.
    fn ensure_matrix(&mut self) {
        if self.matrix.is_some() {
            return;
        }

        // Use fast_qr to generate the matrix
        let qr = fast_qr::QRBuilder::new(self.content.as_bytes())
            .build()
            .expect("Failed to generate QR code");

        let dimension = qr.size;
        let mut data = Vec::with_capacity(dimension * dimension);

        // Access data (assuming compacted row-major)
        for y in 0..dimension {
            for x in 0..dimension {
                // fast_qr::QRCode usually indexes via [index] or [row][col]
                // We'll rely on index operator if available or calculate offset for compacted data
                // Assuming it's compacted:
                let idx = y * dimension + x;
                // Safe check
                let is_dark = if idx < qr.data.len() {
                    qr.data[idx].value()
                } else {
                    false
                };
                data.push(is_dark);
            }
        }

        self.matrix = Some(QrMatrix { data, dimension });
    }

    /// Creates or updates the GPU texture.
    fn ensure_texture(&mut self, device: &wgpu::Device) {
        if self.texture.is_some() && !self.dirty {
            return;
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("barcode texture"),
            size: wgpu::Extent3d {
                width: self.size,
                height: self.size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        self.texture = Some(texture);
    }
}

impl Source for BarcodeSource {
    fn resolve(&mut self, ctx: &mut RenderContext<'_>) -> wgpu::TextureView {
        // Ensure we have the matrix
        self.ensure_matrix();
        
        // Ensure we have a texture
        self.ensure_texture(ctx.device);
        
        let texture = self.texture.as_ref().expect("Texture should exist");
        let matrix = self.matrix.as_ref().expect("Matrix should exist");

        if self.dirty {
            // Generate pixel data on CPU
            // We must respect 256-byte alignment per row for write_texture
            let width = self.size;
            let height = self.size;
            let bytes_per_pixel = 4;
            let unpadded_bytes_per_row = width * bytes_per_pixel;
            let align = 256;
            let bytes_per_row = (unpadded_bytes_per_row + align - 1) & !(align - 1);
            
            let mut pixels = vec![0u8; (bytes_per_row * height) as usize];
            
            let scale = width as f32 / matrix.dimension as f32;
            
            for y in 0..height {
                for x in 0..width {
                    let mx = (x as f32 / scale) as usize;
                    let my = (y as f32 / scale) as usize;
                    
                    let idx = my * matrix.dimension + mx;
                    let is_dark = matrix.data.get(idx).copied().unwrap_or(false);
                    
                    let row_offset = (y * bytes_per_row) as usize;
                    let pixel_offset = row_offset + (x * bytes_per_pixel) as usize;
                    
                    let color = if is_dark { 0u8 } else { 255u8 };
                    
                    pixels[pixel_offset] = color;     // R
                    pixels[pixel_offset + 1] = color; // G
                    pixels[pixel_offset + 2] = color; // B
                    pixels[pixel_offset + 3] = 255;   // A
                }
            }
            
            // Upload to texture
            ctx.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            
            self.dirty = false;
        }

        texture.create_view(&wgpu::TextureViewDescriptor::default())
    }

    fn dimensions(&self) -> (u32, u32) {
        (self.size, self.size)
    }
}
