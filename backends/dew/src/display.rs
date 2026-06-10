//! The flush boundary: where rasterized regions leave the renderer.
//!
//! Everything above this trait is platform-independent; everything below it
//! is a concrete screen. On desktop the implementation assembles regions
//! into an in-memory framebuffer for snapshot tests; on embedded targets it
//! converts to the panel's pixel format (e.g. RGB565) and streams the
//! region over the display bus, ideally via DMA, without ever holding a
//! full frame in memory.

use crate::compositor::DeviceRegion;

/// Receives rasterized regions from the band compositor.
///
/// Pixels arrive as premultiplied RGBA8 in row-major order with a stride of
/// `region.width * 4` bytes — exactly the scratch pixmap produced by the
/// painter, with no padding between rows.
pub trait DisplayFlush {
    /// Screen size in device pixels as `(width, height)`.
    fn size(&self) -> (u32, u32);

    /// Writes one rasterized region to the screen.
    ///
    /// Called once per scheduled band region, in top-to-bottom order.
    /// `pixels.len()` is exactly `region.area() * 4`.
    fn flush_region(&mut self, region: DeviceRegion, pixels: &[u8]);

    /// Commits all regions flushed since the previous call as one visible
    /// frame.
    ///
    /// Panels that latch on write need no work here; the default is a
    /// no-op.
    fn present(&mut self) {}
}

/// Desktop/test display: a plain RGBA8 framebuffer in memory.
///
/// Used by the offscreen simulator and unit tests to assert on and
/// visually inspect rendered output. Embedded targets do not use this
/// type — it deliberately holds the full frame that dew otherwise avoids
/// allocating.
#[derive(Debug, Clone)]
pub struct BufferDisplay {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl BufferDisplay {
    /// Creates a black, fully transparent framebuffer.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; width as usize * height as usize * 4],
        }
    }

    /// The framebuffer contents as premultiplied RGBA8, row-major.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Encodes the framebuffer as a PNG (un-premultiplying alpha), for
    /// snapshot tests and the offscreen simulator.
    ///
    /// # Panics
    ///
    /// Panics when the framebuffer exceeds `u16::MAX` in either dimension
    /// or PNG encoding fails — both indicate misuse of a test-side type.
    #[must_use]
    pub fn to_png(&self) -> Vec<u8> {
        let width = u16::try_from(self.width).expect("framebuffer width exceeds u16::MAX");
        let height = u16::try_from(self.height).expect("framebuffer height exceeds u16::MAX");
        let mut pixmap = vello_cpu::Pixmap::new(width, height);
        for (target, source) in pixmap
            .data_mut()
            .iter_mut()
            .zip(self.pixels.chunks_exact(4))
        {
            target.r = source[0];
            target.g = source[1];
            target.b = source[2];
            target.a = source[3];
        }
        pixmap
            .into_png()
            .expect("PNG encoding of framebuffer failed")
    }

    /// Returns the premultiplied RGBA8 value at (`x`, `y`).
    ///
    /// # Panics
    ///
    /// Panics when the coordinate is outside the framebuffer.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        assert!(
            x < self.width && y < self.height,
            "pixel ({x}, {y}) outside {}x{} framebuffer",
            self.width,
            self.height
        );
        let offset = (y as usize * self.width as usize + x as usize) * 4;
        self.pixels[offset..offset + 4].try_into().unwrap()
    }
}

impl DisplayFlush for BufferDisplay {
    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn flush_region(&mut self, region: DeviceRegion, pixels: &[u8]) {
        assert_eq!(
            pixels.len() as u64,
            region.area() * 4,
            "region pixel payload does not match region size"
        );
        let row_bytes = region.width as usize * 4;
        for row in 0..region.height as usize {
            let src = &pixels[row * row_bytes..(row + 1) * row_bytes];
            let dst_start =
                ((region.y as usize + row) * self.width as usize + region.x as usize) * 4;
            self.pixels[dst_start..dst_start + row_bytes].copy_from_slice(src);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flushed_region_lands_at_its_offset() {
        let mut display = BufferDisplay::new(8, 4);
        let region = DeviceRegion {
            x: 2,
            y: 1,
            width: 3,
            height: 2,
        };
        let pixels: Vec<u8> = (0..region.area() * 4)
            .map(|i| u8::try_from(i % 256).unwrap())
            .collect();
        display.flush_region(region, &pixels);
        assert_eq!(display.pixel(2, 1), [0, 1, 2, 3]);
        assert_eq!(display.pixel(4, 2), [20, 21, 22, 23]);
        assert_eq!(display.pixel(0, 0), [0, 0, 0, 0]);
        assert_eq!(display.pixel(5, 1), [0, 0, 0, 0]);
    }
}
